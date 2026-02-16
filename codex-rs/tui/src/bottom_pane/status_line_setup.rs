//! Status line configuration view for customizing the TUI status bar.
//!
//! This module provides an interactive picker for selecting which items appear
//! in the status line at the bottom of the terminal. Users can:
//!
//! - **Select items**: Toggle which information is displayed
//! - **Reorder items**: Use left/right arrows to change display order
//! - **Preview changes**: See a live preview of the configured status line
//!
//! # Available Status Line Items
//!
//! - Model information (name, reasoning level)
//! - Directory paths (current dir, project root)
//! - Git information (branch name)
//! - Context usage (remaining %, used %, window size)
//! - Usage limits (5-hour, weekly)
//! - Session info (ID, tokens used)
//! - Application version

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use std::collections::HashSet;
use strum::IntoEnumIterator;
use strum_macros::Display;
use strum_macros::EnumIter;
use strum_macros::EnumString;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::bottom_pane::multi_select_picker::MultiSelectItem;
use crate::bottom_pane::multi_select_picker::MultiSelectPicker;
use crate::i18n::tr;
use crate::render::renderable::Renderable;
use codex_protocol::config_types::Language;

/// Available items that can be displayed in the status line.
///
/// Each variant represents a piece of information that can be shown at the
/// bottom of the TUI. Items are serialized to kebab-case for configuration
/// storage (e.g., `ModelWithReasoning` becomes `model-with-reasoning`).
///
/// Some items are conditionally displayed based on availability:
/// - Git-related items only show when in a git repository
/// - Context/limit items only show when data is available from the API
/// - Session ID only shows after a session has started
#[derive(EnumIter, EnumString, Display, Debug, Clone, Eq, PartialEq)]
#[strum(serialize_all = "kebab_case")]
pub(crate) enum StatusLineItem {
    /// The current model name.
    ModelName,

    /// Model name with reasoning level suffix.
    ModelWithReasoning,

    /// Current working directory path.
    CurrentDir,

    /// Project root directory (if detected).
    ProjectRoot,

    /// Current git branch name (if in a repository).
    GitBranch,

    /// Percentage of context window remaining.
    ContextRemaining,

    /// Percentage of context window used.
    ContextUsed,

    /// Remaining usage on the 5-hour rate limit.
    FiveHourLimit,

    /// Remaining usage on the weekly rate limit.
    WeeklyLimit,

    /// Codex application version.
    CodexVersion,

    /// Total context window size in tokens.
    ContextWindowSize,

    /// Total tokens used in the current session.
    UsedTokens,

    /// Total input tokens consumed.
    TotalInputTokens,

    /// Total output tokens generated.
    TotalOutputTokens,

    /// Full session UUID.
    SessionId,
}

impl StatusLineItem {
    /// User-visible label shown in the popup.
    pub(crate) fn display_name(&self, language: Language) -> &'static str {
        match self {
            StatusLineItem::ModelName => tr(language, "status_line_setup.item_name.model_name"),
            StatusLineItem::ModelWithReasoning => {
                tr(language, "status_line_setup.item_name.model_with_reasoning")
            }
            StatusLineItem::CurrentDir => tr(language, "status_line_setup.item_name.current_dir"),
            StatusLineItem::ProjectRoot => tr(language, "status_line_setup.item_name.project_root"),
            StatusLineItem::GitBranch => tr(language, "status_line_setup.item_name.git_branch"),
            StatusLineItem::ContextRemaining => {
                tr(language, "status_line_setup.item_name.context_remaining")
            }
            StatusLineItem::ContextUsed => tr(language, "status_line_setup.item_name.context_used"),
            StatusLineItem::FiveHourLimit => {
                tr(language, "status_line_setup.item_name.five_hour_limit")
            }
            StatusLineItem::WeeklyLimit => tr(language, "status_line_setup.item_name.weekly_limit"),
            StatusLineItem::CodexVersion => {
                tr(language, "status_line_setup.item_name.codex_version")
            }
            StatusLineItem::ContextWindowSize => {
                tr(language, "status_line_setup.item_name.context_window_size")
            }
            StatusLineItem::UsedTokens => tr(language, "status_line_setup.item_name.used_tokens"),
            StatusLineItem::TotalInputTokens => {
                tr(language, "status_line_setup.item_name.total_input_tokens")
            }
            StatusLineItem::TotalOutputTokens => {
                tr(language, "status_line_setup.item_name.total_output_tokens")
            }
            StatusLineItem::SessionId => tr(language, "status_line_setup.item_name.session_id"),
        }
    }

    /// User-visible description shown in the popup.
    pub(crate) fn description(&self, language: Language) -> &'static str {
        match self {
            StatusLineItem::ModelName => {
                tr(language, "status_line_setup.item_description.model_name")
            }
            StatusLineItem::ModelWithReasoning => tr(
                language,
                "status_line_setup.item_description.model_with_reasoning",
            ),
            StatusLineItem::CurrentDir => {
                tr(language, "status_line_setup.item_description.current_dir")
            }
            StatusLineItem::ProjectRoot => {
                tr(language, "status_line_setup.item_description.project_root")
            }
            StatusLineItem::GitBranch => {
                tr(language, "status_line_setup.item_description.git_branch")
            }
            StatusLineItem::ContextRemaining => tr(
                language,
                "status_line_setup.item_description.context_remaining",
            ),
            StatusLineItem::ContextUsed => {
                tr(language, "status_line_setup.item_description.context_used")
            }
            StatusLineItem::FiveHourLimit => tr(
                language,
                "status_line_setup.item_description.five_hour_limit",
            ),
            StatusLineItem::WeeklyLimit => {
                tr(language, "status_line_setup.item_description.weekly_limit")
            }
            StatusLineItem::CodexVersion => {
                tr(language, "status_line_setup.item_description.codex_version")
            }
            StatusLineItem::ContextWindowSize => tr(
                language,
                "status_line_setup.item_description.context_window_size",
            ),
            StatusLineItem::UsedTokens => {
                tr(language, "status_line_setup.item_description.used_tokens")
            }
            StatusLineItem::TotalInputTokens => tr(
                language,
                "status_line_setup.item_description.total_input_tokens",
            ),
            StatusLineItem::TotalOutputTokens => tr(
                language,
                "status_line_setup.item_description.total_output_tokens",
            ),
            StatusLineItem::SessionId => {
                tr(language, "status_line_setup.item_description.session_id")
            }
        }
    }

    /// Returns an example rendering of this item for the preview.
    ///
    /// These are placeholder values used to show users what each item looks
    /// like in the status line before they confirm their selection.
    pub(crate) fn render(&self, language: Language) -> &'static str {
        match self {
            StatusLineItem::ModelName => "gpt-5.2-codex",
            StatusLineItem::ModelWithReasoning => "gpt-5.2-codex medium",
            StatusLineItem::CurrentDir => "~/project/path",
            StatusLineItem::ProjectRoot => "~/project",
            StatusLineItem::GitBranch => "feat/awesome-feature",
            StatusLineItem::ContextRemaining => {
                tr(language, "status_line_setup.preview.context_remaining")
            }
            StatusLineItem::ContextUsed => tr(language, "status_line_setup.preview.context_used"),
            StatusLineItem::FiveHourLimit => "5h 100%",
            StatusLineItem::WeeklyLimit => tr(language, "status_line_setup.preview.weekly_limit"),
            StatusLineItem::CodexVersion => "v0.93.0",
            StatusLineItem::ContextWindowSize => {
                tr(language, "status_line_setup.preview.context_window_size")
            }
            StatusLineItem::UsedTokens => tr(language, "status_line_setup.preview.used_tokens"),
            StatusLineItem::TotalInputTokens => {
                tr(language, "status_line_setup.preview.total_input_tokens")
            }
            StatusLineItem::TotalOutputTokens => {
                tr(language, "status_line_setup.preview.total_output_tokens")
            }
            StatusLineItem::SessionId => "019c19bd-ceb6-73b0-adc8-8ec0397b85cf",
        }
    }
}

/// Interactive view for configuring which items appear in the status line.
///
/// Wraps a [`MultiSelectPicker`] with status-line-specific behavior:
/// - Pre-populates items from current configuration
/// - Shows a live preview of the configured status line
/// - Emits [`AppEvent::StatusLineSetup`] on confirmation
/// - Emits [`AppEvent::StatusLineSetupCancelled`] on cancellation
pub(crate) struct StatusLineSetupView {
    /// The underlying multi-select picker widget.
    picker: MultiSelectPicker,
}

impl StatusLineSetupView {
    /// Creates a new status line setup view.
    ///
    /// # Arguments
    ///
    /// * `status_line_items` - Currently configured item IDs (in display order),
    ///   or `None` to start with all items disabled
    /// * `app_event_tx` - Event sender for dispatching configuration changes
    ///
    /// Items from `status_line_items` are shown first (in order) and marked as
    /// enabled. Remaining items are appended and marked as disabled.
    pub(crate) fn new(
        status_line_items: Option<&[String]>,
        app_event_tx: AppEventSender,
        language: Language,
    ) -> Self {
        let mut used_ids = HashSet::new();
        let mut items = Vec::new();

        if let Some(selected_items) = status_line_items.as_ref() {
            for id in *selected_items {
                let Ok(item) = id.parse::<StatusLineItem>() else {
                    continue;
                };
                let item_id = item.to_string();
                if !used_ids.insert(item_id.clone()) {
                    continue;
                }
                items.push(Self::status_line_select_item(item, true, language));
            }
        }

        for item in StatusLineItem::iter() {
            let item_id = item.to_string();
            if used_ids.contains(&item_id) {
                continue;
            }
            items.push(Self::status_line_select_item(item, false, language));
        }

        let preview_language = language;

        Self {
            picker: MultiSelectPicker::builder(
                tr(language, "status_line_setup.title").to_string(),
                Some(tr(language, "status_line_setup.subtitle").to_string()),
                app_event_tx,
            )
            .instructions(vec![tr(language, "status_line_setup.instructions").into()])
            .items(items)
            .enable_ordering()
            .on_preview(move |items| {
                let preview = items
                    .iter()
                    .filter(|item| item.enabled)
                    .filter_map(|item| item.id.parse::<StatusLineItem>().ok())
                    .map(|item| item.render(preview_language))
                    .collect::<Vec<_>>()
                    .join(" · ");
                if preview.is_empty() {
                    None
                } else {
                    Some(Line::from(preview))
                }
            })
            .on_confirm(|ids, app_event| {
                let items = ids
                    .iter()
                    .map(|id| id.parse::<StatusLineItem>())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or_default();
                app_event.send(AppEvent::StatusLineSetup { items });
            })
            .on_cancel(|app_event| {
                app_event.send(AppEvent::StatusLineSetupCancelled);
            })
            .build(),
        }
    }

    /// Converts a [`StatusLineItem`] into a [`MultiSelectItem`] for the picker.
    fn status_line_select_item(
        item: StatusLineItem,
        enabled: bool,
        language: Language,
    ) -> MultiSelectItem {
        MultiSelectItem {
            id: item.to_string(),
            name: item.display_name(language).to_string(),
            description: Some(item.description(language).to_string()),
            enabled,
        }
    }
}

impl BottomPaneView for StatusLineSetupView {
    fn handle_key_event(&mut self, key_event: crossterm::event::KeyEvent) {
        self.picker.handle_key_event(key_event);
    }

    fn is_complete(&self) -> bool {
        self.picker.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.picker.close();
        CancellationEvent::Handled
    }
}

impl Renderable for StatusLineSetupView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.picker.render(area, buf)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.picker.desired_height(width)
    }
}
