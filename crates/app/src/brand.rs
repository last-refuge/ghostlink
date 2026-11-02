//! Embedded brand artwork, shared across renders without filesystem access.
use gpui::{Image, ImageFormat};
use std::sync::{Arc, OnceLock};

pub fn logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../../branding/ghostlink-logo-v1.png").to_vec(),
        ))
    })
    .clone()
}
