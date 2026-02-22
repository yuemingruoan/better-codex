use std::collections::HashMap;
use std::collections::VecDeque;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::bottom_pane::popup_consts::MAX_POPUP_ROWS;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::bottom_pane::scroll_state::ScrollState;
use crate::bottom_pane::selection_popup_common::GenericDisplayRow;
use crate::bottom_pane::selection_popup_common::measure_rows_height;
use crate::bottom_pane::selection_popup_common::render_rows;
use crate::bottom_pane::selection_popup_common::wrap_styled_line;
use crate::i18n::tr;
use crate::i18n::tr_args;
use crate::render::renderable::Renderable;
use crate::style::user_message_style;
use codex_core::protocol::Op;
use codex_protocol::config_types::Language;
use codex_protocol::request_user_input::RequestUserInputAnswer;
use codex_protocol::request_user_input::RequestUserInputEvent;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_protocol::request_user_input::RequestUserInputResponse;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Options,
    Notes,
}

#[derive(Default)]
struct AnswerState {
    options_state: ScrollState,
    answer_committed: bool,
    note: String,
}

pub(crate) struct RequestUserInputOverlay {
    app_event_tx: AppEventSender,
    request: RequestUserInputEvent,
    queue: VecDeque<RequestUserInputEvent>,
    answers: Vec<AnswerState>,
    current_idx: usize,
    focus: Focus,
    done: bool,
    language: Language,
}

impl RequestUserInputOverlay {
    pub(crate) fn new(request: RequestUserInputEvent, app_event_tx: AppEventSender) -> Self {
        let mut overlay = Self {
            app_event_tx,
            request,
            queue: VecDeque::new(),
            answers: Vec::new(),
            current_idx: 0,
            focus: Focus::Options,
            done: false,
            language: Language::En,
        };
        overlay.reset_for_request();
        overlay
    }

    pub(crate) fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    fn reset_for_request(&mut self) {
        self.current_idx = 0;
        self.focus = Focus::Options;
        self.answers = self
            .request
            .questions
            .iter()
            .map(|question| {
                let options_len = self.options_len_for_question(question);
                let mut options_state = ScrollState::new();
                if options_len > 0 {
                    options_state.selected_idx = Some(0);
                }
                AnswerState {
                    options_state,
                    answer_committed: false,
                    note: String::new(),
                }
            })
            .collect();
        self.ensure_focus_available();
    }

    fn question_count(&self) -> usize {
        self.request.questions.len()
    }

    fn current_question(&self) -> Option<&RequestUserInputQuestion> {
        self.request.questions.get(self.current_idx)
    }

    fn current_answer(&self) -> Option<&AnswerState> {
        self.answers.get(self.current_idx)
    }

    fn current_answer_mut(&mut self) -> Option<&mut AnswerState> {
        self.answers.get_mut(self.current_idx)
    }

    fn options_for_question(
        &self,
        question: &RequestUserInputQuestion,
    ) -> Vec<RequestUserInputQuestionOption> {
        let mut options = question.options.clone().unwrap_or_default();
        if question.is_other {
            options.push(RequestUserInputQuestionOption {
                label: tr(self.language, "request_user_input.other_option.label").to_string(),
                description: tr(self.language, "request_user_input.other_option.description")
                    .to_string(),
            });
        }
        options
    }

    fn options_len_for_question(&self, question: &RequestUserInputQuestion) -> usize {
        self.options_for_question(question).len()
    }

    fn has_options(&self) -> bool {
        self.current_question()
            .is_some_and(|question| self.options_len_for_question(question) > 0)
    }

    fn note_is_empty(&self, idx: usize) -> bool {
        self.answers
            .get(idx)
            .is_none_or(|answer| answer.note.trim().is_empty())
    }

    fn unanswered_count(&self) -> usize {
        self.request
            .questions
            .iter()
            .enumerate()
            .filter(|(idx, question)| {
                let has_options = self.options_len_for_question(question) > 0;
                let note_empty = self.note_is_empty(*idx);
                if has_options {
                    let committed = self
                        .answers
                        .get(*idx)
                        .is_some_and(|answer| answer.answer_committed);
                    !committed && note_empty
                } else {
                    note_empty
                }
            })
            .count()
    }

    fn focus_is_options(&self) -> bool {
        self.focus == Focus::Options
    }

    fn focus_is_notes(&self) -> bool {
        self.focus == Focus::Notes
    }

    fn ensure_focus_available(&mut self) {
        if !self.has_options() {
            self.focus = Focus::Notes;
        }
    }

    fn move_selection_up(&mut self) {
        let options_len = self
            .current_question()
            .map(|question| self.options_len_for_question(question))
            .unwrap_or(0);
        if options_len == 0 {
            return;
        }
        if let Some(answer) = self.current_answer_mut() {
            answer.options_state.move_up_wrap(options_len);
            answer.answer_committed = false;
            answer
                .options_state
                .ensure_visible(options_len, MAX_POPUP_ROWS);
        }
    }

    fn move_selection_down(&mut self) {
        let options_len = self
            .current_question()
            .map(|question| self.options_len_for_question(question))
            .unwrap_or(0);
        if options_len == 0 {
            return;
        }
        if let Some(answer) = self.current_answer_mut() {
            answer.options_state.move_down_wrap(options_len);
            answer.answer_committed = false;
            answer
                .options_state
                .ensure_visible(options_len, MAX_POPUP_ROWS);
        }
    }

    fn move_question(&mut self, next: bool) {
        let count = self.question_count();
        if count == 0 {
            return;
        }
        if next {
            self.current_idx = (self.current_idx + 1) % count;
        } else if self.current_idx == 0 {
            self.current_idx = count - 1;
        } else {
            self.current_idx -= 1;
        }
        self.ensure_focus_available();
    }

    fn clear_current_note(&mut self) {
        if let Some(answer) = self.current_answer_mut() {
            answer.note.clear();
        }
    }

    fn append_note_char(&mut self, c: char) {
        if let Some(answer) = self.current_answer_mut() {
            answer.note.push(c);
        }
    }

    fn pop_note_char(&mut self) {
        if let Some(answer) = self.current_answer_mut() {
            answer.note.pop();
        }
    }

    fn commit_current_answer(&mut self) {
        if self.has_options() {
            if let Some(answer) = self.current_answer_mut() {
                answer.answer_committed = true;
            }
        }
    }

    fn go_next_or_submit(&mut self) {
        if self.question_count() == 0 {
            self.submit_answers();
            return;
        }
        self.commit_current_answer();
        if self.current_idx + 1 >= self.question_count() {
            self.submit_answers();
        } else {
            self.current_idx += 1;
            self.ensure_focus_available();
        }
    }

    fn current_option_rows(&self) -> Vec<GenericDisplayRow> {
        let Some(question) = self.current_question() else {
            return Vec::new();
        };
        self.options_for_question(question)
            .into_iter()
            .enumerate()
            .map(|(idx, option)| {
                let description = (!option.description.trim().is_empty())
                    .then_some(option.description.to_string());
                GenericDisplayRow {
                    name: format!("{}. {}", idx + 1, option.label),
                    display_shortcut: None,
                    match_indices: None,
                    description,
                    wrap_indent: None,
                }
            })
            .collect()
    }

    fn handle_note_key(&mut self, key_event: KeyEvent) -> bool {
        match key_event {
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.pop_note_char();
                true
            }
            KeyEvent {
                code: KeyCode::Delete,
                ..
            } => {
                self.clear_current_note();
                true
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.append_note_char(c);
                true
            }
            _ => false,
        }
    }

    fn build_result_answers(&self) -> HashMap<String, RequestUserInputAnswer> {
        let mut answers: HashMap<String, RequestUserInputAnswer> = HashMap::new();
        for (idx, question) in self.request.questions.iter().enumerate() {
            let mut values = Vec::new();
            if let Some(state) = self.answers.get(idx) {
                let options = self.options_for_question(question);
                if state.answer_committed
                    && let Some(selected_idx) = state.options_state.selected_idx
                    && let Some(option) = options.get(selected_idx)
                {
                    values.push(option.label.clone());
                }
                let note = state.note.trim().to_string();
                if !note.is_empty() {
                    values.push(note);
                }
            }
            answers.insert(
                question.id.clone(),
                RequestUserInputAnswer { answers: values },
            );
        }
        answers
    }

    fn submit_answers(&mut self) {
        let response = RequestUserInputResponse {
            answers: self.build_result_answers(),
        };
        self.app_event_tx
            .send(AppEvent::CodexOp(Op::UserInputAnswer {
                id: self.request.turn_id.clone(),
                response,
            }));

        if let Some(next) = self.queue.pop_front() {
            self.request = next;
            self.reset_for_request();
        } else {
            self.done = true;
        }
    }

    fn notes_placeholder_for_current_question(&self) -> String {
        if self.has_options() {
            tr(self.language, "request_user_input.placeholder.notes").to_string()
        } else {
            tr(self.language, "request_user_input.placeholder.answer").to_string()
        }
    }

    fn option_empty_text(&self) -> String {
        tr(self.language, "request_user_input.empty.options").to_string()
    }

    fn footer_hint_line(&self) -> Line<'static> {
        let enter_hint_key = if self.current_idx + 1 >= self.question_count() {
            "request_user_input.footer.enter_submit_all"
        } else {
            "request_user_input.footer.enter_submit_answer"
        };
        let tab_key = if self.focus_is_notes()
            && self
                .current_answer()
                .is_some_and(|answer| !answer.note.trim().is_empty())
        {
            "request_user_input.footer.tab_or_esc_clear_notes"
        } else {
            "request_user_input.footer.tab_add_notes"
        };
        let text = format!(
            "{} | {} | {}",
            tr(self.language, tab_key),
            tr(self.language, enter_hint_key),
            tr(self.language, "request_user_input.footer.esc_interrupt")
        );
        Line::from(text.dim())
    }

    fn render_wrapped_line(
        line: Line<'static>,
        x: u16,
        y: &mut u16,
        max_y: u16,
        width: u16,
        buf: &mut Buffer,
    ) {
        let wrapped = wrap_styled_line(&line, width);
        for wrapped_line in wrapped {
            if *y >= max_y {
                return;
            }
            Paragraph::new(wrapped_line).render(
                Rect {
                    x,
                    y: *y,
                    width,
                    height: 1,
                },
                buf,
            );
            *y = y.saturating_add(1);
        }
    }
}

impl BottomPaneView for RequestUserInputOverlay {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if matches!(key_event.kind, KeyEventKind::Release) {
            return;
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.move_question(true),
            KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => self.move_question(false),
            KeyEvent {
                code: KeyCode::Up, ..
            } if self.focus_is_options() => self.move_selection_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            } if self.focus_is_options() => self.move_selection_down(),
            KeyEvent {
                code: KeyCode::Up, ..
            } if self.focus_is_notes() && self.has_options() => self.move_selection_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            } if self.focus_is_notes() && self.has_options() => self.move_selection_down(),
            KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if self.focus_is_options() {
                    self.focus = Focus::Notes;
                } else if self.has_options() {
                    self.focus = Focus::Options;
                }
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => self.go_next_or_submit(),
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if self.focus_is_options()
                && !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                if let Some(idx) = c
                    .to_digit(10)
                    .map(|digit| digit as usize)
                    .and_then(|digit| digit.checked_sub(1))
                {
                    let options_len = self
                        .current_question()
                        .map(|question| self.options_len_for_question(question))
                        .unwrap_or(0);
                    if idx < options_len {
                        if let Some(answer) = self.current_answer_mut() {
                            answer.options_state.selected_idx = Some(idx);
                            answer.answer_committed = true;
                        }
                        self.go_next_or_submit();
                        return;
                    }
                }
                self.focus = Focus::Notes;
                let _ = self.handle_note_key(key_event);
            }
            _ if self.focus_is_notes() => {
                let _ = self.handle_note_key(key_event);
            }
            _ => {}
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        if self.focus_is_notes()
            && self
                .current_answer()
                .is_some_and(|answer| !answer.note.trim().is_empty())
        {
            self.clear_current_note();
            return CancellationEvent::Handled;
        }

        self.app_event_tx.send(AppEvent::CodexOp(Op::Interrupt));
        self.done = true;
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if pasted.is_empty() {
            return false;
        }
        if self.focus_is_options() {
            self.focus = Focus::Notes;
        }
        if let Some(answer) = self.current_answer_mut() {
            answer.note.push_str(&pasted);
            return true;
        }
        false
    }

    fn try_consume_user_input_request(
        &mut self,
        request: RequestUserInputEvent,
    ) -> Option<RequestUserInputEvent> {
        self.queue.push_back(request);
        None
    }
}

impl Renderable for RequestUserInputOverlay {
    fn desired_height(&self, width: u16) -> u16 {
        let content_width = width.saturating_sub(2).max(1);
        let question_count = self.question_count();
        let unanswered = self.unanswered_count().to_string();
        let title_line = Line::from(vec![
            "request_user_input".bold(),
            format!(" [{}]", self.request.call_id).dim(),
            " ".into(),
            tr_args(
                self.language,
                "request_user_input.progress.unanswered",
                &[("count", unanswered.as_str())],
            )
            .dim(),
        ]);
        let mut height: u16 = wrap_styled_line(&title_line, content_width).len() as u16;

        let progress = if question_count == 0 {
            tr(self.language, "request_user_input.progress.none").to_string()
        } else {
            tr_args(
                self.language,
                "request_user_input.progress.question",
                &[
                    ("index", (self.current_idx + 1).to_string().as_str()),
                    ("total", question_count.to_string().as_str()),
                ],
            )
            .to_string()
        };
        height = height
            .saturating_add(wrap_styled_line(&Line::from(progress), content_width).len() as u16);

        if let Some(question) = self.current_question() {
            if !question.header.trim().is_empty() {
                height = height.saturating_add(
                    wrap_styled_line(&Line::from(question.header.clone()), content_width).len()
                        as u16,
                );
            }
            if !question.question.trim().is_empty() {
                height = height.saturating_add(
                    wrap_styled_line(&Line::from(question.question.clone()), content_width).len()
                        as u16,
                );
            }
        }

        let rows = self.current_option_rows();
        if rows.is_empty() {
            height = height.saturating_add(1);
        } else {
            let option_state = self
                .current_answer()
                .map(|answer| answer.options_state)
                .unwrap_or_default();
            height = height.saturating_add(measure_rows_height(
                &rows,
                &option_state,
                MAX_POPUP_ROWS,
                content_width,
            ));
        }

        // notes label + notes input + footer hint
        height
            .saturating_add(3)
            .max(6)
            .min(MAX_POPUP_ROWS as u16 + 10)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        Block::default()
            .style(user_message_style())
            .render(area, buf);

        let x = area.x.saturating_add(1);
        let width = area.width.saturating_sub(2).max(1);
        let max_y = area.y.saturating_add(area.height);
        let mut y = area.y;

        let unanswered = self.unanswered_count().to_string();
        let title = Line::from(vec![
            "request_user_input".bold(),
            format!(" [{}]", self.request.call_id).dim(),
            " ".into(),
            tr_args(
                self.language,
                "request_user_input.progress.unanswered",
                &[("count", unanswered.as_str())],
            )
            .dim(),
        ]);
        Self::render_wrapped_line(title, x, &mut y, max_y, width, buf);

        let progress = if self.question_count() == 0 {
            tr(self.language, "request_user_input.progress.none").to_string()
        } else {
            tr_args(
                self.language,
                "request_user_input.progress.question",
                &[
                    ("index", (self.current_idx + 1).to_string().as_str()),
                    ("total", self.question_count().to_string().as_str()),
                ],
            )
            .to_string()
        };
        Self::render_wrapped_line(Line::from(progress.dim()), x, &mut y, max_y, width, buf);

        if let Some(question) = self.current_question() {
            if !question.header.trim().is_empty() {
                Self::render_wrapped_line(
                    Line::from(question.header.clone()),
                    x,
                    &mut y,
                    max_y,
                    width,
                    buf,
                );
            }
            if !question.question.trim().is_empty() {
                Self::render_wrapped_line(
                    Line::from(question.question.clone()),
                    x,
                    &mut y,
                    max_y,
                    width,
                    buf,
                );
            }
        }

        if y >= max_y {
            return;
        }

        let option_rows = self.current_option_rows();
        let option_state = self
            .current_answer()
            .map(|answer| answer.options_state)
            .unwrap_or_default();
        let option_height = if option_rows.is_empty() {
            1
        } else {
            measure_rows_height(&option_rows, &option_state, MAX_POPUP_ROWS, width)
                .min(max_y.saturating_sub(y))
        };
        let option_area = Rect {
            x,
            y,
            width,
            height: option_height,
        };
        render_rows(
            option_area,
            buf,
            &option_rows,
            &option_state,
            MAX_POPUP_ROWS,
            self.option_empty_text().as_str(),
        );
        y = y.saturating_add(option_height);

        if y >= max_y {
            return;
        }

        let note_label =
            Line::from(tr(self.language, "request_user_input.placeholder.notes").to_string());
        Self::render_wrapped_line(note_label, x, &mut y, max_y, width, buf);

        if y >= max_y {
            return;
        }

        let (note_text, note_is_secret) = self
            .current_question()
            .and_then(|question| {
                self.current_answer()
                    .map(|answer| (answer.note.as_str(), question.is_secret))
            })
            .unwrap_or(("", false));
        let note_display = if note_text.is_empty() {
            if self.focus_is_options() && self.has_options() {
                tr(
                    self.language,
                    "request_user_input.placeholder.select_option",
                )
                .dim()
            } else {
                self.notes_placeholder_for_current_question().dim()
            }
            .to_string()
        } else if note_is_secret {
            "•".repeat(note_text.chars().count())
        } else {
            note_text.to_string()
        };
        let note_prefix = if self.focus_is_notes() {
            "> ".cyan().to_string()
        } else {
            "  ".to_string()
        };
        let note_line = Line::from(format!("{note_prefix}{note_display}"));
        Paragraph::new(note_line).render(
            Rect {
                x,
                y,
                width,
                height: 1,
            },
            buf,
        );
        y = y.saturating_add(1);

        if y >= max_y {
            return;
        }

        let footer_hint = if self.question_count() > 1 {
            self.footer_hint_line()
        } else {
            standard_popup_hint_line(self.language)
        };
        Paragraph::new(footer_hint).render(
            Rect {
                x,
                y,
                width,
                height: 1,
            },
            buf,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::error::TryRecvError;
    use tokio::sync::mpsc::unbounded_channel;

    fn sample_request_user_input_event(call_id: &str, turn_id: &str) -> RequestUserInputEvent {
        RequestUserInputEvent {
            call_id: call_id.to_string(),
            turn_id: turn_id.to_string(),
            questions: vec![RequestUserInputQuestion {
                id: "bug_type".to_string(),
                header: "Bug类型".to_string(),
                question: "请选择你遇到的异常类型".to_string(),
                is_other: true,
                is_secret: false,
                options: Some(vec![
                    RequestUserInputQuestionOption {
                        label: "无法选中".to_string(),
                        description: "点击选项后没有高亮或状态变化".to_string(),
                    },
                    RequestUserInputQuestionOption {
                        label: "无法提交".to_string(),
                        description: "可以选中但无法确认提交".to_string(),
                    },
                ]),
            }],
        }
    }

    fn sample_multi_question_event(call_id: &str, turn_id: &str) -> RequestUserInputEvent {
        RequestUserInputEvent {
            call_id: call_id.to_string(),
            turn_id: turn_id.to_string(),
            questions: vec![
                RequestUserInputQuestion {
                    id: "auth_method".to_string(),
                    header: "Auth".to_string(),
                    question: "Choose auth method".to_string(),
                    is_other: false,
                    is_secret: false,
                    options: Some(vec![
                        RequestUserInputQuestionOption {
                            label: "OAuth".to_string(),
                            description: "Browser auth".to_string(),
                        },
                        RequestUserInputQuestionOption {
                            label: "API Key".to_string(),
                            description: "Static secret".to_string(),
                        },
                    ]),
                },
                RequestUserInputQuestion {
                    id: "account_id".to_string(),
                    header: "Account".to_string(),
                    question: "Provide account id".to_string(),
                    is_other: false,
                    is_secret: false,
                    options: None,
                },
            ],
        }
    }

    #[test]
    fn enter_submits_selected_option() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let mut overlay = RequestUserInputOverlay::new(
            sample_request_user_input_event("rui-call-1", "turn-1"),
            AppEventSender::new(tx_raw),
        );

        overlay.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(overlay.is_complete(), "overlay should close after submit");
        let event = rx.try_recv().expect("expected one AppEvent");
        let AppEvent::CodexOp(Op::UserInputAnswer { id, response }) = event else {
            panic!("expected Op::UserInputAnswer");
        };
        assert_eq!(id, "turn-1");
        let answer = response
            .answers
            .get("bug_type")
            .expect("expected answer for question");
        assert_eq!(answer.answers, vec!["无法选中".to_string()]);
        assert!(rx.try_recv().is_err(), "expected no extra AppEvents");
    }

    #[test]
    fn esc_clears_note_before_interrupt() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let mut overlay = RequestUserInputOverlay::new(
            sample_request_user_input_event("rui-call-2", "turn-2"),
            AppEventSender::new(tx_raw),
        );

        overlay.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        overlay.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(
            overlay.current_answer().map(|answer| answer.note.as_str()),
            Some("x")
        );

        // Esc is routed via BottomPane::on_ctrl_c before handle_key_event.
        let cancellation = overlay.on_ctrl_c();
        assert_eq!(cancellation, CancellationEvent::Handled);
        assert_eq!(
            overlay.current_answer().map(|answer| answer.note.as_str()),
            Some("")
        );
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(!overlay.is_complete(), "overlay should remain active");

        let cancellation = overlay.on_ctrl_c();
        assert_eq!(cancellation, CancellationEvent::Handled);
        assert!(overlay.is_complete(), "overlay should close on second esc");
        let event = rx.try_recv().expect("expected interrupt event");
        let AppEvent::CodexOp(Op::Interrupt) = event else {
            panic!("expected Op::Interrupt");
        };
    }

    #[test]
    fn multi_question_enter_advances_then_submits() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let mut overlay = RequestUserInputOverlay::new(
            sample_multi_question_event("rui-call-3", "turn-3"),
            AppEventSender::new(tx_raw),
        );

        overlay.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        overlay.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            !overlay.is_complete(),
            "overlay should stay open before final question submit"
        );
        assert_eq!(
            overlay
                .current_question()
                .map(|question| question.id.as_str()),
            Some("account_id")
        );

        overlay.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        overlay.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        overlay.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(
            overlay.is_complete(),
            "overlay should close after final submit"
        );
        let event = rx.try_recv().expect("expected one AppEvent");
        let AppEvent::CodexOp(Op::UserInputAnswer { id, response }) = event else {
            panic!("expected Op::UserInputAnswer");
        };
        assert_eq!(id, "turn-3");
        assert_eq!(
            response
                .answers
                .get("auth_method")
                .map(|answer| answer.answers.clone()),
            Some(vec!["API Key".to_string()])
        );
        assert_eq!(
            response
                .answers
                .get("account_id")
                .map(|answer| answer.answers.clone()),
            Some(vec!["a1".to_string()])
        );
        assert!(rx.try_recv().is_err(), "expected no extra AppEvents");
    }

    #[test]
    fn queued_request_is_consumed_after_submit() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let first = sample_request_user_input_event("rui-call-4a", "turn-4a");
        let second = sample_request_user_input_event("rui-call-4b", "turn-4b");
        let mut overlay = RequestUserInputOverlay::new(first, AppEventSender::new(tx_raw));

        assert!(
            overlay.try_consume_user_input_request(second).is_none(),
            "second request should be consumed into queue"
        );

        overlay.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            !overlay.is_complete(),
            "overlay should continue with queued request"
        );
        assert_eq!(overlay.request.turn_id, "turn-4b");

        let first_event = rx.try_recv().expect("expected first submit event");
        let AppEvent::CodexOp(Op::UserInputAnswer { id, .. }) = first_event else {
            panic!("expected first Op::UserInputAnswer");
        };
        assert_eq!(id, "turn-4a");

        overlay.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            overlay.is_complete(),
            "overlay should close after last request"
        );

        let second_event = rx.try_recv().expect("expected second submit event");
        let AppEvent::CodexOp(Op::UserInputAnswer { id, .. }) = second_event else {
            panic!("expected second Op::UserInputAnswer");
        };
        assert_eq!(id, "turn-4b");
        assert!(rx.try_recv().is_err(), "expected no extra AppEvents");
    }
}
