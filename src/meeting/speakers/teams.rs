//! Microsoft Teams (the new desktop app, `com.microsoft.teams2`).
//!
//! Evidence (third-party, published; never observed by this project): the
//! macOS app is Chromium-based and exposes `AXDOMIdentifier` and
//! `AXDOMClassList`; its meeting window holds the `hangup-button` control
//! (danswett/streamdeck-teams-control #4 and J3tto/callcontrol #3, both read
//! live on macOS). In the meeting window, each gallery tile is an
//! `AXMenuItem` labelled "<Name>, …, Context menu is available" (the user's
//! own: "Myself video, <Name>, …"), and the tile being heard holds a frame
//! with the class `vdi-frame-occlusion`; the compact view, titled "Meeting
//! compact view | <meeting>", shows an `AXGroup` "<Name> is speaking"
//! (EnviousWispr's macOS runs of 2026-10-27/28). One web-client project saw
//! that class never change in a 13-minute meeting (Vexa), so it may say
//! nothing at all on some builds: then every line stays `Them`.
//!
//! Tile labels are read only for tiles that hold the frame, and only in a
//! verified meeting window; their trailing states are parsed in English, and
//! any other ending names nobody.

use super::ax::{AxNode, Budget, Call, NoCall, Reading, has_class};
use super::names;

pub const BUNDLE_ID: &str = "com.microsoft.teams2";
const HANGUP: &str = "hangup-button";
const COMPACT_PREFIX: &str = "Meeting compact view | ";
const FRAME: &str = "vdi-frame-occlusion";
const SELF_TILE_PREFIX: &str = "Myself video, ";
const SPEAKING_SUFFIX: &str = " is speaking";
/// Context menus and text fields hold no tiles.
const SKIP_ROLES: &[&str] = &["AXMenu", "AXMenuBar", "AXTextArea", "AXTextField"];
/// How a gallery tile's label ends.
const TILE_ENDINGS: &[&str] = &["Context menu is available", "Has context menu"];
const MORE_OPTIONS: &str = " More options";
/// States Teams lists after a name; `muted` is checked on its own.
const TILE_STATES: &[&str] = &[
    "muted",
    "unmuted",
    "organizer",
    "organiser",
    "co-organizer",
    "presenter",
    "attendee",
    "guest",
    "meeting guest",
];
const TILE_STATE_PREFIXES: &[&str] = &["video is ", "fit to ", "has control of"];
/// Pieces that mean the label is not one name, wherever they sit.
const NOT_NAMES: &[&str] = &[
    "pinned",
    "spotlighted",
    "spotlight",
    "hand raised",
    "raised hand",
    "presenting",
    "speaking",
    "you",
];
const NAME_TAGS: &[&str] = &[
    " (Guest)",
    " (External)",
    " (Unverified)",
    " Unverified",
    " (Organizer)",
];
const SELF_TAGS: &[&str] = &[" (You)", " (Me)", " (me)", " (you)"];

struct Window<N> {
    window: N,
    compact_of: Option<String>,
    title: String,
    hangup: bool,
    /// Gallery tiles holding the speaking frame.
    framed: Vec<N>,
    /// The frame outside any tile: someone unidentifiable is marked.
    loose_frame: bool,
    tiles: usize,
    groups: Vec<N>,
}

pub fn read<N: AxNode>(windows: Vec<N>, budget: &mut Budget) -> Result<Call<N>, NoCall> {
    let mut meeting = Vec::new();
    for window in windows {
        let title = window.title().unwrap_or_default();
        let found = walk(window, title, budget)?;
        if found.hangup || found.compact_of.is_some() {
            meeting.push(found);
        }
    }
    let (compact, main): (Vec<_>, Vec<_>) = meeting
        .into_iter()
        .partition(|window| window.compact_of.is_some());
    if main.len() > 1 || compact.len() > 1 {
        return Err(NoCall::Unknown);
    }
    if let (Some(main), Some(compact)) = (main.first(), compact.first())
        && compact.compact_of.as_deref() != Some(main.title.as_str())
    {
        return Err(NoCall::Unknown);
    }
    if main
        .iter()
        .chain(&compact)
        .all(|window| window.tiles == 0 && window.groups.is_empty())
    {
        return Err(NoCall::Absent);
    }
    let mut speaking = Vec::new();
    let mut uncertain = false;
    for window in main.iter().chain(&compact) {
        uncertain |= window.loose_frame;
        for tile in &window.framed {
            match label(tile).as_deref().map(parse_tile) {
                Some(Some(Tile { is_self: true, .. })) => {}
                Some(Some(Tile {
                    name, muted: false, ..
                })) => speaking.push(name),
                _ => uncertain = true,
            }
        }
        for group in &window.groups {
            if let Some(label) = label(group)
                && let Some(name) = label.strip_suffix(SPEAKING_SUFFIX)
            {
                match clean_name(name) {
                    Some((name, false)) => speaking.push(name),
                    Some((_, true)) => {}
                    None => uncertain = true,
                }
            }
        }
    }
    speaking.dedup();
    speaking.sort();
    speaking.dedup();
    let reading = match (uncertain, speaking.len()) {
        (true, _) => Reading::Uncertain,
        (false, 0) => Reading::Silent,
        (false, 1) => Reading::Speaking(speaking.remove(0)),
        (false, _) => Reading::Uncertain,
    };
    let region = main
        .into_iter()
        .chain(compact)
        .next()
        .map(|window| window.window)
        .ok_or(NoCall::Absent)?;
    Ok(Call { region, reading })
}

fn walk<N: AxNode>(window: N, title: String, budget: &mut Budget) -> Result<Window<N>, NoCall> {
    let compact_of = title.strip_prefix(COMPACT_PREFIX).map(str::to_owned);
    let mut found = Window {
        window: window.clone(),
        compact_of,
        title,
        hangup: false,
        framed: Vec::new(),
        loose_frame: false,
        tiles: 0,
        groups: Vec::new(),
    };
    // Each node with the index of the tile it sits in.
    let mut tiles: Vec<(N, bool)> = Vec::new();
    let mut stack = vec![(window, 0, None::<usize>)];
    while let Some((node, depth, tile)) = stack.pop() {
        if !budget.visit() {
            return Err(NoCall::Unknown);
        }
        let role = node.role().unwrap_or_default();
        if SKIP_ROLES.contains(&role.as_str()) {
            continue;
        }
        found.hangup |= node.dom_id().as_deref() == Some(HANGUP);
        let tile = if role == "AXMenuItem" {
            tiles.push((node.clone(), false));
            Some(tiles.len() - 1)
        } else {
            tile
        };
        if has_class(&node, FRAME) {
            match tile {
                Some(index) => tiles[index].1 = true,
                None => found.loose_frame = true,
            }
        }
        if role == "AXGroup" && found.compact_of.is_some() {
            found.groups.push(node.clone());
        }
        // Every element of the window is read or the reading is unknown: an
        // unread subtree could hold a second marked tile.
        let children = budget
            .children_at(&node, depth, Budget::MAX_DEPTH)
            .ok_or(NoCall::Unknown)?;
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1, tile));
        }
    }
    found.tiles = tiles.len();
    found.framed = tiles
        .into_iter()
        .filter_map(|(tile, framed)| framed.then_some(tile))
        .collect();
    Ok(found)
}

fn label<N: AxNode>(node: &N) -> Option<String> {
    node.title()
        .filter(|text| !text.trim().is_empty())
        .or_else(|| node.description())
}

#[derive(Debug, Eq, PartialEq)]
struct Tile {
    name: String,
    muted: bool,
    is_self: bool,
}

/// "Ana Ruiz (Guest), muted, Context menu is available" or "Ana Ruiz,
/// unmuted More options": the name, then states. Read from the end, so a
/// name holding a comma (`Richardson, Bob`) survives; a label that does not
/// end like a tile, or holds anything else, names nobody.
fn parse_tile(label: &str) -> Option<Tile> {
    let label = label.trim();
    let (label, is_self) = match label.strip_prefix(SELF_TILE_PREFIX) {
        Some(rest) => (rest, true),
        None => (label, false),
    };
    let (label, ended) = match label.strip_suffix(MORE_OPTIONS) {
        Some(rest) => (rest, true),
        None => (label, false),
    };
    let mut parts = label.split(',').map(str::trim).collect::<Vec<_>>();
    if !ended {
        if !parts.last().is_some_and(|last| TILE_ENDINGS.contains(last)) {
            return None;
        }
        parts.pop();
    }
    let mut muted = false;
    while let Some(last) = parts.last() {
        let lower = last.to_lowercase();
        if TILE_STATES.contains(&lower.as_str())
            || TILE_STATE_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix))
        {
            muted |= lower == "muted";
            parts.pop();
        } else {
            break;
        }
    }
    let plausible = match parts.as_slice() {
        [_] => true,
        [last, first] => [last, first].iter().all(|part| {
            part.chars()
                .next()
                .is_some_and(|first| !first.is_lowercase())
        }),
        _ => false,
    };
    if !plausible
        || parts
            .iter()
            .any(|part| NOT_NAMES.contains(&part.to_lowercase().as_str()))
        || parts
            .first()
            .is_some_and(|first| first.to_lowercase().starts_with("content shared by"))
    {
        return None;
    }
    let (name, tagged_self) = clean_name(&parts.join(", "))?;
    Some(Tile {
        name,
        muted,
        is_self: is_self || tagged_self,
    })
}

/// A display name without Teams' tags, and whether a tag marks the user.
fn clean_name(raw: &str) -> Option<(String, bool)> {
    let mut name = raw.trim().to_owned();
    let mut is_self = false;
    loop {
        if let Some(tag) = SELF_TAGS.iter().find(|tag| name.ends_with(*tag)) {
            name.truncate(name.len() - tag.len());
            is_self = true;
        } else if let Some(tag) = NAME_TAGS.iter().find(|tag| name.ends_with(*tag)) {
            name.truncate(name.len() - tag.len());
        } else {
            break;
        }
    }
    names::clean(&name).map(|name| (name, is_self))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::ax::fake::{Builder, Node};
    use super::*;

    fn budget() -> Budget {
        Budget::new(10_000, Duration::from_secs(5))
    }

    fn tile(label: &str, speaking: bool) -> Builder {
        let mut tile = Node::element("AXMenuItem").description(label);
        let frame = if speaking { FRAME } else { "vdi-occlusion" };
        tile = tile.child(Node::element("AXGroup").child(Node::element("AXGroup").class(frame)));
        tile
    }

    fn meeting(title: &str, tiles: impl IntoIterator<Item = Builder>) -> Node {
        Node::element("AXWindow")
            .title(title)
            .child(
                Node::element("AXGroup")
                    .child(
                        Node::element("AXButton")
                            .id("microphone-button")
                            .description("Mute mic"),
                    )
                    .child(Node::element("AXButton").id(HANGUP).description("Leave")),
            )
            .child(Node::element("AXGroup").children(tiles))
            .build()
    }

    fn reading(windows: Vec<Node>) -> Result<Reading, NoCall> {
        read(windows, &mut budget()).map(|call| call.reading)
    }

    #[test]
    fn the_framed_tile_in_the_meeting_window_names_the_speaker() {
        let window = meeting(
            "Weekly sync | Microsoft Teams",
            [
                tile("Ana Ruiz, Context menu is available", true),
                tile(
                    "Ben Okafor (External), muted, Context menu is available",
                    false,
                ),
                tile("Myself video, Cara Diaz, Context menu is available", false),
            ],
        );
        assert_eq!(
            reading(vec![window.clone()]),
            Ok(Reading::Speaking("Ana Ruiz".into()))
        );
        // Unframed tiles' labels were never asked for.
        assert!(window.reads().iter().all(|text| !text.contains("Ben")));
    }

    #[test]
    fn names_keep_their_comma_and_lose_teams_tags() {
        for (label, name) in [
            (
                "Richardson, Bob, Context menu is available",
                "Richardson, Bob",
            ),
            (
                "Ana Ruiz (Guest), unmuted, Context menu is available",
                "Ana Ruiz",
            ),
            ("Ana Ruiz, unmuted More options", "Ana Ruiz"),
            (
                "Ana Ruiz Unverified, Organizer, Video is off, Has context menu",
                "Ana Ruiz",
            ),
        ] {
            let window = meeting("Sync | Microsoft Teams", [tile(label, true)]);
            assert_eq!(
                reading(vec![window]),
                Ok(Reading::Speaking(name.into())),
                "{label}"
            );
        }
    }

    #[test]
    fn unrecognized_tile_labels_are_uncertain_rather_than_guessed() {
        for label in [
            // Another language's states.
            "Ana Ruiz, stummgeschaltet, Kontextmenü ist verfügbar",
            // A state Hex doesn't know, which would otherwise join the name.
            "Ana Ruiz, Pinned, Context menu is available",
            "Ana Ruiz, hand raised, Context menu is available",
            "Content shared by Ana Ruiz, Context menu is available",
            "Ana, Ben, Cara, Context menu is available",
            "Ana Ruiz, speaking, Context menu is available",
            "",
        ] {
            let window = meeting("Sync | Microsoft Teams", [tile(label, true)]);
            assert_eq!(reading(vec![window]), Ok(Reading::Uncertain), "{label}");
        }
    }

    #[test]
    fn a_muted_framed_tile_is_uncertain_and_the_users_own_frame_is_silence() {
        let muted = meeting(
            "Sync | Microsoft Teams",
            [tile("Ana Ruiz, muted, Context menu is available", true)],
        );
        assert_eq!(reading(vec![muted]), Ok(Reading::Uncertain));
        let mine = meeting(
            "Sync | Microsoft Teams",
            [
                tile("Ana Ruiz, Context menu is available", false),
                tile("Myself video, Cara Diaz, Context menu is available", true),
            ],
        );
        assert_eq!(reading(vec![mine]), Ok(Reading::Silent));
        let tagged = meeting(
            "Sync | Microsoft Teams",
            [tile("Cara Diaz (You), Context menu is available", true)],
        );
        assert_eq!(reading(vec![tagged]), Ok(Reading::Silent));
    }

    #[test]
    fn two_frames_or_a_frame_outside_a_tile_is_uncertain() {
        let two = meeting(
            "Sync | Microsoft Teams",
            [
                tile("Ana Ruiz, Context menu is available", true),
                tile("Ben Okafor, Context menu is available", true),
            ],
        );
        assert_eq!(reading(vec![two]), Ok(Reading::Uncertain));
        let loose = Node::element("AXWindow")
            .title("Sync | Microsoft Teams")
            .child(Node::element("AXButton").id(HANGUP))
            .child(tile("Ana Ruiz, Context menu is available", false))
            .child(Node::element("AXGroup").class(FRAME))
            .build();
        assert_eq!(reading(vec![loose]), Ok(Reading::Uncertain));
    }

    #[test]
    fn menus_and_windows_without_the_meeting_controls_are_never_read() {
        let chat = Node::element("AXWindow")
            .title("Chat | Microsoft Teams")
            .child(tile("Dana Wu, Context menu is available", true))
            .build();
        assert_eq!(reading(vec![chat.clone()]), Err(NoCall::Absent));
        assert!(chat.reads().iter().all(|text| !text.contains("Dana")));

        let menu = Node::element("AXWindow")
            .title("Sync | Microsoft Teams")
            .child(Node::element("AXButton").id(HANGUP))
            .child(tile("Ana Ruiz, Context menu is available", false))
            .child(
                Node::element("AXMenu")
                    .child(tile("Pin for everyone, Context menu is available", true)),
            )
            .build();
        assert_eq!(reading(vec![menu]), Ok(Reading::Silent));
    }

    #[test]
    fn the_compact_view_names_its_speaker_with_its_main_window_or_alone() {
        let compact = |title: &str, label: &str| {
            Node::element("AXWindow")
                .title(&format!("{COMPACT_PREFIX}{title}"))
                .child(Node::element("AXGroup").description(label))
                .build()
        };
        assert_eq!(
            reading(vec![compact("Sync", "Ana Ruiz (Guest) is speaking")]),
            Ok(Reading::Speaking("Ana Ruiz".into()))
        );
        assert_eq!(
            reading(vec![
                meeting("Sync", [tile("Ana Ruiz, Context menu is available", true)]),
                compact("Sync", "Ana Ruiz is speaking"),
            ]),
            Ok(Reading::Speaking("Ana Ruiz".into()))
        );
        // Windows of two different meetings.
        assert_eq!(
            reading(vec![
                meeting("Sync", [tile("Ana Ruiz, Context menu is available", true)]),
                compact("Standup", "Ben Okafor is speaking"),
            ]),
            Err(NoCall::Unknown)
        );
        // The windows disagree.
        assert_eq!(
            reading(vec![
                meeting("Sync", [tile("Ana Ruiz, Context menu is available", true)]),
                compact("Sync", "Ben Okafor is speaking"),
            ]),
            Ok(Reading::Uncertain)
        );
        assert_eq!(
            reading(vec![compact("Sync", "You are speaking")]),
            Ok(Reading::Silent)
        );
    }

    #[test]
    fn two_meeting_windows_are_ambiguous() {
        let one = meeting("Sync", [tile("Ana Ruiz, Context menu is available", true)]);
        let two = meeting(
            "Standup",
            [tile("Ben Okafor, Context menu is available", true)],
        );
        assert_eq!(reading(vec![one, two]), Err(NoCall::Unknown));
    }

    #[test]
    fn a_second_mark_hex_could_not_read_makes_the_reading_unknown() {
        use super::super::ax::fake::{crowd, nested};
        let ada = || tile("Ana Ruiz, Context menu is available", true);
        let ben = || tile("Ben Okafor, Context menu is available", true);
        // Ben's tile sits deeper than the walk goes.
        let deep = meeting("Sync", [ada(), nested(Budget::MAX_DEPTH, ben())]);
        assert_eq!(reading(vec![deep]), Err(NoCall::Unknown));
        // The app fails to list the children holding Ben's tile.
        let failed = meeting(
            "Sync",
            [ada(), Node::element("AXGroup").unreadable().child(ben())],
        );
        assert_eq!(reading(vec![failed]), Err(NoCall::Unknown));
        // More children than one element may report.
        let crowded = meeting(
            "Sync",
            [
                ada(),
                Node::element("AXGroup").children(crowd(Budget::MAX_CHILDREN, ben())),
            ],
        );
        assert_eq!(reading(vec![crowded]), Err(NoCall::Unknown));
        // Deep but complete trees still read, and an empty element at the
        // depth limit leaves nothing out.
        let complete = meeting("Sync", [nested(Budget::MAX_DEPTH - 4, ada())]);
        assert_eq!(
            reading(vec![complete]),
            Ok(Reading::Speaking("Ana Ruiz".into()))
        );
        let leaf = meeting(
            "Sync",
            [
                ada(),
                nested(Budget::MAX_DEPTH - 3, Node::element("AXGroup")),
            ],
        );
        assert_eq!(
            reading(vec![leaf]),
            Ok(Reading::Speaking("Ana Ruiz".into()))
        );
    }

    #[test]
    fn a_walk_that_runs_out_of_budget_is_unknown() {
        let window = meeting("Sync", [tile("Ana Ruiz, Context menu is available", true)]);
        assert_eq!(
            read(vec![window], &mut Budget::new(4, Duration::from_secs(5)))
                .map(|call| call.reading),
            Err(NoCall::Unknown)
        );
    }
}
