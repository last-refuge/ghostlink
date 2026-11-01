//! Embedded brand artwork, shared across renders without filesystem access.
use gpui::{Image, ImageFormat, Pixels, Size, px, size};
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

pub fn illustration() -> Arc<Image> {
    static ILLUSTRATION: OnceLock<Arc<Image>> = OnceLock::new();
    ILLUSTRATION
        .get_or_init(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../../branding/ghostlink-illustration-v1.png").to_vec(),
            ))
        })
        .clone()
}

pub fn welcome_image_size(viewport: Size<Pixels>, sidebar: bool) -> Size<Pixels> {
    const ASPECT: f32 = 1672. / 941.;
    let available_width =
        (f32::from(viewport.width) - if sidebar { 248. } else { 0. } - 48.).max(0.);
    let available_height = (f32::from(viewport.height) - 244.).max(0.);
    let width = available_width.min(640.).min(available_height * ASPECT);
    size(px(width), px(width / ASPECT))
}
