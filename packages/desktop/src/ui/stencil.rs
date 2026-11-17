//! The stencil code and line colour that mark which group a host belongs to.

use gpui_kit::Rgba;
use shared::HostGroupRecord;

const fn line(value: u32, dark_text: bool) -> (Rgba, bool) {
    let fill = Rgba {
        r: ((value >> 16) & 0xff) as f32 / 255.,
        g: ((value >> 8) & 0xff) as f32 / 255.,
        b: (value & 0xff) as f32 / 255.,
        a: 1.,
    };
    (fill, dark_text)
}

// Container line colours, each with >= 4.5:1 contrast for its stencil text.
const LINE_COLORS: [(Rgba, bool); 4] = [
    line(0xa33b28, false),
    line(0x2a648f, false),
    line(0x336a41, false),
    line(0xc9952e, true),
];
const UNGROUPED_LINE: (Rgba, bool) = line(0x2a2f36, false);

/// Groups take line colours in name order, so neighbours in the Connections
/// list never share a colour until there are more groups than colours.
// ponytail: renaming a group can shift colours; store a per-group colour if
// users want them pinned.
pub fn line_color(group_id: &str, groups: &[HostGroupRecord]) -> (Rgba, bool) {
    let mut sorted: Vec<&HostGroupRecord> = groups.iter().collect();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    match sorted.iter().position(|group| group.id == group_id) {
        Some(index) => LINE_COLORS[index % LINE_COLORS.len()],
        None => UNGROUPED_LINE,
    }
}

pub fn group_prefix(name: &str) -> String {
    let letters: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(3)
        .collect::<String>()
        .to_uppercase();
    if letters.is_empty() {
        "GRP".to_string()
    } else {
        letters
    }
}

#[cfg(test)]
mod tests {
    use super::{group_prefix, line_color, LINE_COLORS, UNGROUPED_LINE};
    use shared::HostGroupRecord;

    #[test]
    fn prefixes_and_line_colours_follow_group_names() {
        assert_eq!(group_prefix("prod-eu"), "PRO");
        assert_eq!(group_prefix("測試"), "GRP");
        let group = |id: &str, name: &str| HostGroupRecord {
            id: id.to_string(),
            name: name.to_string(),
        };
        let groups = [group("2", "beta"), group("1", "alpha")];
        assert_eq!(line_color("1", &groups), LINE_COLORS[0]);
        assert_eq!(line_color("2", &groups), LINE_COLORS[1]);
        assert_eq!(line_color("missing", &groups), UNGROUPED_LINE);
    }
}
