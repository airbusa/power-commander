//! KDE: the user's Qt style (System Settings' Application Style) draws the
//! controls, through `pc-qtstyle` beside the app, into images the GUI shows.

use std::collections::HashMap;
use std::io;
use std::process::{Command, Stdio};

/// An image in device pixels, straight RGBA.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Drawn {
    /// The style's name: breeze, fusion, …
    pub style: String,
    /// Whether its colours are dark.
    pub dark: bool,
    /// By name, as `checkbox.on.hovered`, `button.default.pressed.left`.
    pub images: HashMap<String, Picture>,
}

/// The controls as the Qt style draws them, `scale` device pixels to a
/// pixel, buttons `button_height` pixels high. Blocks, some 70 ms.
pub fn draw(scale: f32, button_height: u32) -> io::Result<Drawn> {
    let helper = std::env::current_exe()?.with_file_name("pc-qtstyle");
    let output = Command::new(helper)
        .arg(scale.to_string())
        .arg(button_height.to_string())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("pc-qtstyle: {}", output.status)));
    }
    parse(&output.stdout)
}

/// Lines, an image's pixels after its line, "end" last.
fn parse(mut data: &[u8]) -> io::Result<Drawn> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("pc-qtstyle: {what}"));
    let mut drawn = Drawn { style: String::new(), dark: false, images: HashMap::new() };
    while let Some(end) = data.iter().position(|&byte| byte == b'\n') {
        let line = std::str::from_utf8(&data[..end]).map_err(|_| bad("not text"))?;
        data = &data[end + 1..];
        let words: Vec<&str> = line.split(' ').collect();
        match words[..] {
            ["style", name] => drawn.style = name.to_owned(),
            ["dark", dark] => drawn.dark = dark == "1",
            ["image", name, width, height] => {
                let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
                    return Err(bad(line));
                };
                let size = width as usize * height as usize * 4;
                let rgba = data.get(..size).ok_or_else(|| bad("an image cut short"))?.to_vec();
                data = &data[size..];
                drawn.images.insert(name.to_owned(), Picture { width, height, rgba });
            }
            ["end"] => return Ok(drawn),
            _ => {}
        }
    }
    Err(bad("cut short"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_helpers_output() {
        let mut data = b"pc-qtstyle 1\nstyle fusion\ndark 1\nimage checkbox.on.normal 2 1\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 10, 11, 12, 13]);
        data.extend_from_slice(b"end\n");
        let drawn = parse(&data).unwrap();
        assert_eq!((drawn.style.as_str(), drawn.dark), ("fusion", true));
        let picture = &drawn.images["checkbox.on.normal"];
        assert_eq!((picture.width, picture.height, picture.rgba.len()), (2, 1, 8));
        // Cut short: no "end", or fewer pixels than said.
        assert!(parse(&data[..data.len() - 4]).is_err());
        assert!(parse(b"image a 2 2\n\x00\x00end\n").is_err());
    }
}
