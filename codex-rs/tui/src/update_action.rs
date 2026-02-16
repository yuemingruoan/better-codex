/// 更新动作：始终引导用户打开 Releases 页面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    OpenReleasePage,
    NpmGlobalLatest,
    BunGlobalLatest,
    BrewUpgrade,
}

impl UpdateAction {
    pub const RELEASE_PAGE_URL: &'static str =
        "https://github.com/yuemingruoan/better-codex/releases";
    /// Returns the list of command-line arguments for invoking the update.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        match self {
            UpdateAction::NpmGlobalLatest => ("npm", &["install", "-g", "@openai/codex"]),
            UpdateAction::BunGlobalLatest => ("bun", &["install", "-g", "@openai/codex"]),
            UpdateAction::BrewUpgrade => ("brew", &["upgrade", "--cask", "codex"]),
            UpdateAction::OpenReleasePage => ("", &[]),
        }
    }

    pub fn release_url(self) -> &'static str {
        Self::RELEASE_PAGE_URL
    }
}

#[cfg(not(debug_assertions))]
pub(crate) fn get_update_action() -> Option<UpdateAction> {
    Some(UpdateAction::OpenReleasePage)
}
