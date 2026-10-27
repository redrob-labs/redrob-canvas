//! Help → project links (the Redrob website, this app's releases, its GitHub repository and the
//! issue tracker). The commands return the URL; the frontend opens it.

use serde_json::{Value, json};

use super::*;

pub const WEBSITE_URL: &str = "https://redrob.ai";
pub const GITHUB_URL: &str = "https://github.com/redrob-labs/redrob-canvas";
pub const ISSUES_URL: &str = "https://github.com/redrob-labs/redrob-canvas/issues";

/// This app's releases page (downloads and release notes).
pub fn app_page_url() -> String {
    format!("{GITHUB_URL}/releases")
}

/// This app's source repository.
pub fn github_url() -> String {
    GITHUB_URL.to_string()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "help.issues", "Report an Issue…", ["Help"], None, "{} → {url} bugs, feedback and feature requests on GitHub", always, |_, _| url(ISSUES_URL.into())),
        cmd!(query "help.website", "Redrob website", ["Help"], None, "{} → {url}", always, |_, _| url(WEBSITE_URL.into())),
        cmd!(query "help.appPage", "Redrob Canvas releases", ["Help"], None, "{} → {url} downloads and release notes", always, |_, _| url(app_page_url())),
        cmd!(query "help.github", "Redrob Canvas on GitHub", ["Help"], None, "{} → {url} source code, issues and releases", always, |_, _| url(github_url())),
        cmd!(query "help.links", "Links", [], None, "{} → {issues, website, appPage, github}", always, |_, _| {
            Ok(json!({ "issues": ISSUES_URL, "website": WEBSITE_URL, "appPage": app_page_url(), "github": github_url() }))
        }),
    ]
}

fn url(u: String) -> Result<Value> {
    Ok(json!({ "url": u }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let mut s = Session::new();
        assert_eq!(s.execute("help.issues", &json!({})).unwrap()["url"], "https://github.com/redrob-labs/redrob-canvas/issues");
        assert_eq!(s.execute("help.appPage", &json!({})).unwrap()["url"], "https://github.com/redrob-labs/redrob-canvas/releases");
        assert_eq!(s.execute("help.github", &json!({})).unwrap()["url"], "https://github.com/redrob-labs/redrob-canvas");
        assert_eq!(s.execute("help.links", &json!({})).unwrap()["website"], "https://redrob.ai");
    }
}
