/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Open the Releases page in the default browser.
    OpenReleasePage,
}

impl From<UpdateAction> for codex_tui::update_action::UpdateAction {
    fn from(action: UpdateAction) -> Self {
        match action {
            UpdateAction::OpenReleasePage => {
                codex_tui::update_action::UpdateAction::OpenReleasePage
            }
        }
    }
}

impl UpdateAction {
    pub const RELEASE_PAGE_URL: &'static str =
        "https://github.com/yuemingruoan/better-codex/releases";

    pub fn release_url(self) -> &'static str {
        Self::RELEASE_PAGE_URL
    }
}

#[cfg(not(debug_assertions))]
pub(crate) fn get_update_action() -> Option<UpdateAction> {
    Some(UpdateAction::OpenReleasePage)
}
