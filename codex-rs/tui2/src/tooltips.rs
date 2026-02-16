use crate::i18n::tr_list;
use codex_core::features::FEATURES;
use codex_protocol::config_types::Language;
use lazy_static::lazy_static;
use rand::Rng;

const ANNOUNCEMENT_TIP_URL: &str =
    "https://raw.githubusercontent.com/openai/codex/main/announcement_tip.toml";

lazy_static! {
    static ref TOOLTIPS_EN: Vec<String> = tr_list(Language::En, "tooltips.items").to_vec();
    static ref TOOLTIPS_ZH: Vec<String> = tr_list(Language::ZhCn, "tooltips.items").to_vec();
    static ref ALL_TOOLTIPS_EN: Vec<String> = {
        let mut tips = Vec::new();
        tips.extend(TOOLTIPS_EN.iter().cloned());
        tips.extend(beta_tooltips().into_iter().map(str::to_string));
        tips
    };
    static ref ALL_TOOLTIPS_ZH: Vec<String> = {
        let mut tips = Vec::new();
        tips.extend(TOOLTIPS_ZH.iter().cloned());
        tips.extend(beta_tooltips().into_iter().map(str::to_string));
        tips
    };
}

fn beta_tooltips() -> Vec<&'static str> {
    FEATURES
        .iter()
        .filter_map(|spec| spec.stage.experimental_announcement())
        .collect()
}

/// Pick a random tooltip to show to the user when starting Codex.
pub(crate) fn random_tooltip(language: Language) -> Option<String> {
    if let Some(announcement) = announcement::fetch_announcement_tip() {
        return Some(announcement);
    }
    let mut rng = rand::rng();
    pick_tooltip(&mut rng, language).map(str::to_string)
}

fn pick_tooltip<R: Rng + ?Sized>(rng: &mut R, language: Language) -> Option<&'static str> {
    let tooltips = match language {
        Language::ZhCn => &*ALL_TOOLTIPS_ZH,
        Language::En => &*ALL_TOOLTIPS_EN,
    };
    if tooltips.is_empty() {
        None
    } else {
        tooltips
            .get(rng.random_range(0..tooltips.len()))
            .map(String::as_str)
    }
}

pub(crate) mod announcement {
    use crate::tooltips::ANNOUNCEMENT_TIP_URL;
    use crate::version::CODEX_CLI_VERSION;
    use chrono::NaiveDate;
    use chrono::Utc;
    use regex_lite::Regex;
    use serde::Deserialize;
    use std::sync::OnceLock;
    use std::thread;
    use std::time::Duration;

    static ANNOUNCEMENT_TIP: OnceLock<Option<String>> = OnceLock::new();

    /// Prewarm the cache of the announcement tip.
    pub(crate) fn prewarm() {
        let _ = thread::spawn(|| ANNOUNCEMENT_TIP.get_or_init(init_announcement_tip_in_thread));
    }

    /// Fetch the announcement tip, return None if the prewarm is not done yet.
    pub(crate) fn fetch_announcement_tip() -> Option<String> {
        ANNOUNCEMENT_TIP
            .get()
            .cloned()
            .flatten()
            .and_then(|raw| parse_announcement_tip_toml(&raw))
    }

    #[derive(Debug, Deserialize)]
    struct AnnouncementTipRaw {
        content: String,
        from_date: Option<String>,
        to_date: Option<String>,
        version_regex: Option<String>,
        target_app: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct AnnouncementTipDocument {
        announcements: Vec<AnnouncementTipRaw>,
    }

    #[derive(Debug)]
    struct AnnouncementTip {
        content: String,
        from_date: Option<NaiveDate>,
        to_date: Option<NaiveDate>,
        version_regex: Option<Regex>,
        target_app: String,
    }

    fn init_announcement_tip_in_thread() -> Option<String> {
        thread::spawn(blocking_init_announcement_tip)
            .join()
            .ok()
            .flatten()
    }

    fn blocking_init_announcement_tip() -> Option<String> {
        let response = reqwest::blocking::Client::new()
            .get(ANNOUNCEMENT_TIP_URL)
            .timeout(Duration::from_millis(2000))
            .send()
            .ok()?;
        response.error_for_status().ok()?.text().ok()
    }

    pub(crate) fn parse_announcement_tip_toml(text: &str) -> Option<String> {
        let doc: AnnouncementTipDocument = toml::from_str(text).ok()?;
        let today = Utc::now().date_naive();
        doc.announcements
            .into_iter()
            .map(|raw| AnnouncementTip {
                content: raw.content,
                from_date: raw
                    .from_date
                    .as_ref()
                    .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
                to_date: raw
                    .to_date
                    .as_ref()
                    .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
                version_regex: raw
                    .version_regex
                    .as_ref()
                    .and_then(|regex| Regex::new(regex).ok()),
                target_app: raw.target_app.unwrap_or_else(|| "codex-cli".to_string()),
            })
            .filter(|tip| tip.target_app == "codex-cli")
            .filter(|tip| tip.from_date.is_none_or(|from_date| from_date <= today))
            .filter(|tip| tip.to_date.is_none_or(|to_date| today <= to_date))
            .filter(|tip| {
                tip.version_regex
                    .as_ref()
                    .is_none_or(|regex| regex.is_match(CODEX_CLI_VERSION))
            })
            .map(|tip| tip.content)
            .next()
    }
}
