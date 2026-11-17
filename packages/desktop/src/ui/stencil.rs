//! The stencil code and line colour that say which environment a connection
//! reaches. They are fixed per environment, so production always looks the same.

use gpui_kit::Rgba;
use shared::Environment;

const fn line(value: u32, dark_text: bool) -> (Rgba, bool) {
    let fill = Rgba {
        r: ((value >> 16) & 0xff) as f32 / 255.,
        g: ((value >> 8) & 0xff) as f32 / 255.,
        b: (value & 0xff) as f32 / 255.,
        a: 1.,
    };
    (fill, dark_text)
}

/// The fill and whether it takes dark text; each has >= 4.5:1 contrast for
/// its stencil text. Danger rises with the colour: neutral, green, ochre, rust.
pub fn line_color(environment: Environment) -> (Rgba, bool) {
    match environment {
        Environment::Local => line(0x2a2f36, false),
        Environment::Development => line(0x336a41, false),
        Environment::Staging => line(0xc9952e, true),
        Environment::Production => line(0xa33b28, false),
    }
}

/// The three-letter code before a connection's name: `PRD·ORDERS`.
pub fn code(environment: Environment) -> &'static str {
    match environment {
        Environment::Local => "LOC",
        Environment::Development => "DEV",
        Environment::Staging => "STG",
        Environment::Production => "PRD",
    }
}
