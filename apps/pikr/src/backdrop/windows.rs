//! Windows backdrop capture — stub.

pub fn capture(_target_w: u32, _target_h: u32, _sigma: f32, _grayscale: bool) -> Option<Vec<u8>> {
    // TODO: implement via the Windows Desktop Duplication API
    // (`IDXGIOutputDuplication`) — the `windows` crate exposes it.
    None
}
