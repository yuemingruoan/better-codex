use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use codex_async_utils::CancelErr;
use codex_async_utils::OrCancelExt;
use codex_protocol::config_types::Language;
use codex_protocol::user_input::UserInput;
use tokio_util::sync::CancellationToken;
use tracing::error;
use uuid::Uuid;

use crate::codex::TurnContext;
use crate::exec::ExecToolCallOutput;
use crate::exec::SandboxType;
use crate::exec::StreamOutput;
use crate::exec::execute_exec_env;
use crate::exec_env::create_env;
use crate::i18n::tr;
use crate::i18n::tr_args;
use crate::parse_command::parse_command;
use crate::protocol::ErrorEvent;
use crate::protocol::EventMsg;
use crate::protocol::ExecCommandBeginEvent;
use crate::protocol::ExecCommandEndEvent;
use crate::protocol::ExecCommandSource;
use crate::protocol::SandboxPolicy;
use crate::protocol::SddGitAction;
use crate::protocol::TurnStartedEvent;
use crate::protocol::WarningEvent;
use crate::sandboxing::ExecRequest;
use crate::state::TaskKind;
use crate::tools::format_exec_output_str;

use super::SessionTask;
use super::SessionTaskContext;

const SDD_GIT_TIMEOUT_MS: u64 = 5 * 60 * 1000;
const SDD_BRANCH_PREFIX: &str = "sdd/";

#[derive(Clone)]
pub(crate) struct SddGitTask {
    action: SddGitAction,
}

impl SddGitTask {
    pub(crate) fn new(action: SddGitAction) -> Self {
        Self { action }
    }
}

#[async_trait]
impl SessionTask for SddGitTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        turn_context: Arc<TurnContext>,
        _input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let event = EventMsg::TurnStarted(TurnStartedEvent {
            model_context_window: turn_context.model_context_window(),
            collaboration_mode_kind: turn_context.collaboration_mode.mode,
        });
        let session = session.clone_session();
        session.send_event(turn_context.as_ref(), event).await;

        let result =
            run_sdd_git_action(&session, &turn_context, &self.action, &cancellation_token).await;

        if let Err(message) = result {
            session
                .send_event(
                    turn_context.as_ref(),
                    EventMsg::Error(ErrorEvent {
                        message,
                        codex_error_info: None,
                    }),
                )
                .await;
        }

        None
    }
}

async fn run_sdd_git_action(
    session: &Arc<crate::codex::Session>,
    turn_context: &Arc<TurnContext>,
    action: &SddGitAction,
    cancellation_token: &CancellationToken,
) -> Result<(), String> {
    let language = turn_context.config.language;
    ensure_git_repository(&turn_context.cwd, language)?;

    match action {
        SddGitAction::CreateBranch { name, base } => {
            ensure_sdd_branch(name, language)?;
            ensure_clean_repo(&turn_context.cwd, language)?;

            let current = current_branch(&turn_context.cwd, language)?;
            if current != *base {
                run_git_logged(
                    session,
                    turn_context,
                    vec!["checkout", base],
                    cancellation_token,
                    language,
                )
                .await?;
            }

            run_git_logged(
                session,
                turn_context,
                vec!["checkout", "-b", name],
                cancellation_token,
                language,
            )
            .await?;
        }
        SddGitAction::SwitchBranch { name } => {
            ensure_sdd_branch(name, language)?;

            let current = current_branch(&turn_context.cwd, language)?;
            if current != *name {
                run_git_logged(
                    session,
                    turn_context,
                    vec!["checkout", name],
                    cancellation_token,
                    language,
                )
                .await?;
            }
        }
        SddGitAction::FinalizeMerge {
            name,
            base,
            commit_message,
        } => {
            ensure_sdd_branch(name, language)?;

            let current = current_branch(&turn_context.cwd, language)?;
            let dirty = is_repo_dirty(&turn_context.cwd, language)?;
            if dirty && current != *name {
                return Err(tr(language, "sdd_git.error.dirty_not_on_branch").to_string());
            }
            if current != *name {
                run_git_logged(
                    session,
                    turn_context,
                    vec!["checkout", name],
                    cancellation_token,
                    language,
                )
                .await?;
            }

            let dirty = is_repo_dirty(&turn_context.cwd, language)?;
            if dirty {
                run_git_logged(
                    session,
                    turn_context,
                    vec!["add", "-A"],
                    cancellation_token,
                    language,
                )
                .await?;
                run_git_logged(
                    session,
                    turn_context,
                    vec!["commit", "-m", commit_message],
                    cancellation_token,
                    language,
                )
                .await?;
            } else {
                session
                    .send_event(
                        turn_context.as_ref(),
                        EventMsg::Warning(WarningEvent {
                            message: tr(language, "sdd_git.warning.no_changes_skip_commit")
                                .to_string(),
                        }),
                    )
                    .await;
            }

            let current = current_branch(&turn_context.cwd, language)?;
            if current != *base {
                run_git_logged(
                    session,
                    turn_context,
                    vec!["checkout", base],
                    cancellation_token,
                    language,
                )
                .await?;
            }

            run_git_logged(
                session,
                turn_context,
                vec!["merge", "--no-ff", name],
                cancellation_token,
                language,
            )
            .await?;
        }
        SddGitAction::AbandonBranch { name, base } => {
            ensure_sdd_branch(name, language)?;
            ensure_clean_repo(&turn_context.cwd, language)?;

            let current = current_branch(&turn_context.cwd, language)?;
            if current != *base {
                run_git_logged(
                    session,
                    turn_context,
                    vec!["checkout", base],
                    cancellation_token,
                    language,
                )
                .await?;
            }

            run_git_logged(
                session,
                turn_context,
                vec!["branch", "-D", name],
                cancellation_token,
                language,
            )
            .await?;
        }
    }

    Ok(())
}

fn ensure_sdd_branch(name: &str, language: Language) -> Result<(), String> {
    if !name.starts_with(SDD_BRANCH_PREFIX) {
        let prefix = SDD_BRANCH_PREFIX;
        return Err(tr_args(
            language,
            "sdd_git.error.invalid_prefix",
            &[("prefix", prefix), ("name", name)],
        ));
    }
    if name.len() <= SDD_BRANCH_PREFIX.len() {
        return Err(tr_args(
            language,
            "sdd_git.error.branch_name_invalid",
            &[("name", name)],
        ));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(tr_args(
            language,
            "sdd_git.error.branch_name_whitespace",
            &[("name", name)],
        ));
    }
    if name.contains("..") {
        return Err(tr_args(
            language,
            "sdd_git.error.branch_name_invalid_segment",
            &[("name", name)],
        ));
    }
    Ok(())
}

fn ensure_git_repository(repo: &Path, language: Language) -> Result<(), String> {
    let output = run_git_silent(repo, &["rev-parse", "--is-inside-work-tree"], language)?;
    if output.trim() != "true" {
        return Err(tr(language, "sdd_git.error.not_git_repo").to_string());
    }
    Ok(())
}

fn current_branch(repo: &Path, language: Language) -> Result<String, String> {
    let name = run_git_silent(repo, &["rev-parse", "--abbrev-ref", "HEAD"], language)?;
    if name.is_empty() {
        return Err(tr(language, "sdd_git.error.unknown_branch").to_string());
    }
    Ok(name)
}

fn is_repo_dirty(repo: &Path, language: Language) -> Result<bool, String> {
    let status = run_git_silent(repo, &["status", "--porcelain"], language)?;
    Ok(!status.trim().is_empty())
}

fn ensure_clean_repo(repo: &Path, language: Language) -> Result<(), String> {
    if is_repo_dirty(repo, language)? {
        return Err(tr(language, "sdd_git.error.dirty_workspace").to_string());
    }
    Ok(())
}

fn run_git_silent(repo: &Path, args: &[&str], language: Language) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|err| {
            tr_args(
                language,
                "sdd_git.error.git_exec_failed",
                &[("error", &err.to_string())],
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(tr(language, "sdd_git.error.git_command_failed").to_string());
        }
        return Err(stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn run_git_logged(
    session: &Arc<crate::codex::Session>,
    turn_context: &Arc<TurnContext>,
    args: Vec<&str>,
    cancellation_token: &CancellationToken,
    language: Language,
) -> Result<(), String> {
    let mut command = Vec::with_capacity(args.len() + 1);
    command.push("git".to_string());
    for arg in args {
        command.push(arg.to_string());
    }

    let parsed_cmd = parse_command(&command);
    let call_id = Uuid::new_v4().to_string();

    session
        .send_event(
            turn_context.as_ref(),
            EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: call_id.clone(),
                process_id: None,
                turn_id: turn_context.sub_id.clone(),
                command: command.clone(),
                cwd: turn_context.cwd.clone(),
                parsed_cmd: parsed_cmd.clone(),
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }),
        )
        .await;

    let exec_env = ExecRequest {
        command: command.clone(),
        cwd: turn_context.cwd.clone(),
        env: create_env(
            &turn_context.shell_environment_policy,
            Some(session.conversation_id),
        ),
        network: None,
        expiration: SDD_GIT_TIMEOUT_MS.into(),
        sandbox: SandboxType::None,
        windows_sandbox_level: turn_context.windows_sandbox_level,
        sandbox_permissions: crate::sandboxing::SandboxPermissions::UseDefault,
        justification: None,
        arg0: None,
    };

    let exec_result = execute_exec_env(exec_env, &SandboxPolicy::DangerFullAccess, None)
        .or_cancel(cancellation_token)
        .await;

    match exec_result {
        Err(CancelErr::Cancelled) => {
            let aborted_message = tr(language, "sdd_git.exec.aborted").to_string();
            session
                .send_event(
                    turn_context.as_ref(),
                    EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                        call_id,
                        process_id: None,
                        turn_id: turn_context.sub_id.clone(),
                        command,
                        cwd: turn_context.cwd.clone(),
                        parsed_cmd,
                        source: ExecCommandSource::Agent,
                        interaction_input: None,
                        stdout: String::new(),
                        stderr: aborted_message.clone(),
                        aggregated_output: aborted_message.clone(),
                        exit_code: -1,
                        duration: Duration::ZERO,
                        formatted_output: aborted_message,
                    }),
                )
                .await;
            Err(tr(language, "sdd_git.error.command_cancelled").to_string())
        }
        Ok(Ok(output)) => {
            session
                .send_event(
                    turn_context.as_ref(),
                    EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                        call_id,
                        process_id: None,
                        turn_id: turn_context.sub_id.clone(),
                        command,
                        cwd: turn_context.cwd.clone(),
                        parsed_cmd,
                        source: ExecCommandSource::Agent,
                        interaction_input: None,
                        stdout: output.stdout.text.clone(),
                        stderr: output.stderr.text.clone(),
                        aggregated_output: output.aggregated_output.text.clone(),
                        exit_code: output.exit_code,
                        duration: output.duration,
                        formatted_output: format_exec_output_str(
                            &output,
                            turn_context.truncation_policy,
                        ),
                    }),
                )
                .await;
            if output.exit_code == 0 {
                Ok(())
            } else {
                let message = if output.aggregated_output.text.trim().is_empty() {
                    tr(language, "sdd_git.error.command_failed").to_string()
                } else {
                    output.aggregated_output.text.clone()
                };
                Err(message)
            }
        }
        Ok(Err(err)) => {
            error!("sdd git command failed: {err:?}");
            let message = tr_args(
                language,
                "sdd_git.error.command_failed_detail",
                &[("error", &format!("{err:?}"))],
            );
            let exec_output = ExecToolCallOutput {
                exit_code: -1,
                stdout: StreamOutput::new(String::new()),
                stderr: StreamOutput::new(message.clone()),
                aggregated_output: StreamOutput::new(message.clone()),
                duration: Duration::ZERO,
                timed_out: false,
            };
            session
                .send_event(
                    turn_context.as_ref(),
                    EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                        call_id,
                        process_id: None,
                        turn_id: turn_context.sub_id.clone(),
                        command,
                        cwd: turn_context.cwd.clone(),
                        parsed_cmd,
                        source: ExecCommandSource::Agent,
                        interaction_input: None,
                        stdout: exec_output.stdout.text.clone(),
                        stderr: exec_output.stderr.text.clone(),
                        aggregated_output: exec_output.aggregated_output.text.clone(),
                        exit_code: exec_output.exit_code,
                        duration: exec_output.duration,
                        formatted_output: exec_output.aggregated_output.text.clone(),
                    }),
                )
                .await;
            Err(message)
        }
    }
}
