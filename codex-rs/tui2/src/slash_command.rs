use crate::i18n::tr;
use codex_protocol::config_types::Language;
use strum::IntoEnumIterator;
use strum_macros::AsRefStr;
use strum_macros::EnumIter;
use strum_macros::EnumString;
use strum_macros::IntoStaticStr;

/// Commands that can be invoked by starting a message with a leading slash.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, EnumIter, AsRefStr, IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum SlashCommand {
    // DO NOT ALPHA-SORT! Enum order is presentation order in the popup, so
    // more frequently used commands should be listed first.
    Model,
    Lang,
    Agent,
    Approvals,
    #[strum(serialize = "setup-elevated-sandbox")]
    ElevateSandbox,
    Skills,
    Review,
    New,
    Resume,
    Fork,
    Init,
    Compact,
    // Undo,
    Diff,
    Clean,
    Mention,
    Status,
    Mcp,
    Logout,
    Quit,
    Exit,
    Feedback,
    Rollout,
    TestApproval,
}

impl SlashCommand {
    /// User-visible description shown in the popup.
    pub fn description(self, language: Language) -> &'static str {
        match self {
            SlashCommand::Feedback => tr(language, "slash_command.description.feedback"),
            SlashCommand::New => tr(language, "slash_command.description.new"),
            SlashCommand::Init => tr(language, "slash_command.description.init"),
            SlashCommand::Compact => tr(language, "slash_command.description.compact"),
            SlashCommand::Review => tr(language, "slash_command.description.review"),
            SlashCommand::Resume => tr(language, "slash_command.description.resume"),
            SlashCommand::Fork => tr(language, "slash_command.description.fork"),
            SlashCommand::Quit | SlashCommand::Exit => {
                tr(language, "slash_command.description.exit")
            }
            SlashCommand::Diff => tr(language, "slash_command.description.diff"),
            SlashCommand::Clean => tr(language, "slash_command.description.clean"),
            SlashCommand::Mention => tr(language, "slash_command.description.mention"),
            SlashCommand::Skills => tr(language, "slash_command.description.skills"),
            SlashCommand::Status => tr(language, "slash_command.description.status"),
            SlashCommand::Model => tr(language, "slash_command.description.model"),
            SlashCommand::Lang => tr(language, "slash_command.description.lang"),
            SlashCommand::Agent => tr(language, "slash_command.description.agent"),
            SlashCommand::Approvals => tr(language, "slash_command.description.approvals"),
            SlashCommand::ElevateSandbox => {
                tr(language, "slash_command.description.elevate_sandbox")
            }
            SlashCommand::Mcp => tr(language, "slash_command.description.mcp"),
            SlashCommand::Logout => tr(language, "slash_command.description.logout"),
            SlashCommand::Rollout => tr(language, "slash_command.description.rollout"),
            SlashCommand::TestApproval => tr(language, "slash_command.description.test_approval"),
        }
    }

    /// Command string without the leading '/'. Provided for compatibility with
    /// existing code that expects a method named `command()`.
    pub fn command(self) -> &'static str {
        self.into()
    }

    /// Whether this command can be run while a task is in progress.
    pub fn available_during_task(self) -> bool {
        match self {
            SlashCommand::New
            | SlashCommand::Resume
            | SlashCommand::Fork
            | SlashCommand::Init
            | SlashCommand::Compact
            | SlashCommand::Model
            | SlashCommand::Lang
            | SlashCommand::Agent
            | SlashCommand::Approvals
            | SlashCommand::ElevateSandbox
            | SlashCommand::Review
            | SlashCommand::Logout => false,
            SlashCommand::Diff
            | SlashCommand::Clean
            | SlashCommand::Mention
            | SlashCommand::Skills
            | SlashCommand::Status
            | SlashCommand::Mcp
            | SlashCommand::Feedback
            | SlashCommand::Quit
            | SlashCommand::Exit => true,
            SlashCommand::Rollout => true,
            SlashCommand::TestApproval => true,
        }
    }

    fn is_visible(self) -> bool {
        match self {
            SlashCommand::Rollout | SlashCommand::TestApproval => cfg!(debug_assertions),
            _ => true,
        }
    }
}

/// Return all built-in commands in a Vec paired with their command string.
pub fn built_in_slash_commands() -> Vec<(&'static str, SlashCommand)> {
    SlashCommand::iter()
        .filter(|command| command.is_visible())
        .map(|c| (c.command(), c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::SlashCommand;
    use super::built_in_slash_commands;

    #[test]
    fn agent_command_is_available_and_legacy_agent_entries_are_removed() {
        let commands = built_in_slash_commands();
        assert!(
            commands
                .iter()
                .any(|(name, command)| *name == "agent" && *command == SlashCommand::Agent)
        );
        assert!(commands.iter().all(|(name, _)| {
            *name != "spec"
                && *name != "preset"
                && *name != "collab"
                && *name != "sdd-develop"
                && *name != "sdd-develop-parallels"
        }));
    }
}
