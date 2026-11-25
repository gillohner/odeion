//! The page against the API's own lists: what the page can show or send has to be something
//! the node takes. These read the web UI's files (pm-web), so they run for a change to the UI
//! alone as well (the `pages` job in ci.yml); pm-api's unit tests don't depend on those files.

use pm_api::usage::{Area, Control, ErrorClass, How, Surface, Via, View, Viewport, SCHEMA};
use std::collections::BTreeSet;

/// Claude review of b015b991, Medium: a page the app shows that the inbox doesn't know turns
/// every report sent from it with details into a 400 (and reloading lands on the same page).
#[test]
fn every_page_the_app_shows_is_a_view_a_report_may_name() {
    let js = pm_web::APP_JS;
    let mut pages: Vec<&str> = js
        .match_indices("setView('")
        .filter_map(|(i, m)| js[i + m.len()..].split('\'').next())
        .collect();
    let nav = js
        .split_once("const NAV_VIEWS = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .expect("app.js lists its NAV_VIEWS")
        .0;
    pages.extend(nav.split(',').map(|v| v.trim().trim_matches('\'')));
    assert!(pages.len() > 20, "{pages:?}");
    let unknown: Vec<&&str> = pages
        .iter()
        .filter(|v| !pm_api::feedback::VIEWS.contains(v))
        .collect();
    assert!(unknown.is_empty(), "pages a report can't name: {unknown:?}");
}

/// The browser's recorder (static/usage.js) sends only what this schema knows: every value
/// of its closed lists parses as the enum it is sent as (a value the node refuses would lose
/// the whole batch).
#[test]
fn the_browsers_lists_are_all_in_the_schema() {
    let js = pm_web::USAGE_JS;
    let list = |name: &str| -> Vec<String> {
        let start = format!("const {name} = Object.freeze([");
        let body = js
            .split_once(start.as_str())
            .and_then(|(_, rest)| rest.split_once("])"))
            .unwrap_or_else(|| panic!("usage.js lists its {name}"))
            .0;
        let vals: Vec<String> = body
            .split(',')
            .map(|v| v.trim().trim_matches('\'').to_string())
            .filter(|v| !v.is_empty())
            .collect();
        assert!(!vals.is_empty(), "{name} is empty");
        vals
    };
    fn all<T: serde::de::DeserializeOwned>(name: &str, vals: &[String]) {
        for v in vals {
            serde_json::from_value::<T>(serde_json::Value::String(v.clone()))
                .unwrap_or_else(|_| panic!("usage.js {name} has {v:?}, not in the schema"));
        }
    }
    all::<View>("VIEWS", &list("VIEWS"));
    all::<Area>("AREAS", &list("AREAS"));
    all::<Control>("CONTROLS", &list("CONTROLS"));
    all::<Surface>("SURFACES", &list("SURFACES"));
    all::<Via>("VIAS", &list("VIAS"));
    all::<How>("HOWS", &list("HOWS"));
    all::<Viewport>("VIEWPORTS", &list("VIEWPORTS"));
    all::<ErrorClass>("ERROR_CLASSES", &list("ERROR_CLASSES"));
    let kinds = list("KINDS");
    assert_eq!(
        kinds,
        ["nav", "load", "api", "action", "exception"],
        "S2 sends these kinds only"
    );
    assert!(
        js.contains(&format!("'{SCHEMA}'")),
        "usage.js names the schema it speaks"
    );
}

/// Every page app.js draws is a page the schema can name, and every Settings tab is a
/// control (a new page or tab without one fails here, not as a refused batch).
#[test]
fn every_page_and_settings_tab_of_the_app_is_in_the_schema() {
    let app = pm_web::APP_JS;
    let pages = app
        .split_once("const renderPage = async")
        .and_then(|(_, rest)| rest.split_once("\n  // ---- "))
        .expect("app.js has renderPage")
        .0;
    let mut views: BTreeSet<&str> = pages
        .match_indices("        case '")
        .filter_map(|(i, m)| pages[i + m.len()..].split('\'').next())
        .collect();
    let nav = app
        .split_once("const NAV_VIEWS = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .expect("app.js lists its NAV_VIEWS")
        .0;
    views.extend(nav.split(',').map(|v| v.trim().trim_matches('\'')));
    assert!(views.len() > 25 && views.contains("releases"), "{views:?}");
    for v in &views {
        serde_json::from_value::<View>(serde_json::Value::String(v.replace('-', "_")))
            .unwrap_or_else(|_| panic!("app.js page {v:?} is not a usage View"));
    }
    let tabs = app
        .split_once("const settingsTabs = ")
        .and_then(|(_, rest)| rest.split_once("\n  ];"))
        .expect("app.js has settingsTabs")
        .0;
    let keys: BTreeSet<&str> = tabs
        .match_indices("['")
        .filter_map(|(i, m)| tabs[i + m.len()..].split('\'').next())
        .collect();
    assert!(
        keys.len() > 12 && keys.contains("privacy") && keys.contains("usage"),
        "{keys:?}"
    );
    for k in &keys {
        let k = if *k == "nodehome" { "node_home" } else { k };
        serde_json::from_value::<Control>(serde_json::Value::String(format!("settings_{k}")))
            .unwrap_or_else(|_| panic!("Settings tab {k:?} has no settings_{k} control"));
    }
}
