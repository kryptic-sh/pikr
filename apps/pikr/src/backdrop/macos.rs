//! macOS backdrop capture — stub.

pub fn capture(_target_w: u32, _target_h: u32, _sigma: f32, _grayscale: bool) -> Option<Vec<u8>> {
    // TODO: implement via ScreenCaptureKit (macOS 12.3+) — shell out to
    // `screencapture -x -t png -` works as a stop-gap but adds a click
    // sound and respects screen-recording permission prompts.
    None
}
