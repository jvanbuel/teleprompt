//! A still picture as a clip: what a plugin that draws slides or images
//! hands ffmpeg, so each draws the same way.

use std::path::Path;

use super::Frame;

/// `s` as a value inside an ffmpeg filter, its `\`, `:` and `'` escaped.
pub fn filter_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// A picture fitted to `frame`: filling it and cropped, with `cover`;
/// otherwise letterboxed on `background`.
pub fn fit(frame: &Frame, cover: bool, background: &str) -> String {
    let (w, h) = (frame.width, frame.height);
    if cover {
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
    } else {
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={}",
            filter_value(background)
        )
    }
}

/// The ffmpeg arguments that make `still` a one-second clip at `clip`,
/// at `frame`'s rate, drawn through `filter` (such as [`fit`]'s).
pub fn clip_args(still: &Path, frame: &Frame, filter: &str, clip: &Path) -> Vec<String> {
    let fps = frame.fps.to_string();
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y"]
        .map(String::from)
        .to_vec();
    a.extend(["-loop", "1", "-framerate", &fps, "-t", "1", "-i"].map(String::from));
    a.push(still.display().to_string());
    a.extend(["-vf".into(), format!("{filter},format=yuv420p")]);
    a.extend(["-c:v", "libx264", "-r"].map(String::from));
    a.push(fps);
    a.push(clip.display().to_string());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame {
            width: 1280,
            height: 720,
            fps: 30,
        }
    }

    #[test]
    fn a_still_is_looped_for_a_second_through_its_filter() {
        let a = clip_args(
            Path::new("s.png"),
            &frame(),
            &fit(&frame(), false, "black"),
            Path::new("c.mp4"),
        );
        let at = |flag: &str| a[a.iter().position(|x| x == flag).unwrap() + 1].clone();
        assert_eq!(at("-i"), "s.png");
        assert_eq!(at("-t"), "1");
        assert_eq!(at("-framerate"), "30");
        assert!(at("-vf").starts_with("scale=1280:720:force_original_aspect_ratio=decrease"));
        assert!(at("-vf").ends_with(",format=yuv420p"));
        assert_eq!(a.last().unwrap(), "c.mp4");
    }

    #[test]
    fn a_colour_is_escaped_for_the_filter() {
        assert!(fit(&frame(), false, "#0b0d10").ends_with("color=#0b0d10"));
        assert_eq!(filter_value("a:b'c"), "a\\:b\\'c");
    }
}
