use super::*;
use crate::config::Theme;

#[test]
fn the_protocol_hears_the_theme_as_opaque_rgba() {
    let theme = Theme::default();
    let told = theme::protocol(&theme, "dark", "JetBrains Mono");
    assert_eq!(told.space, (theme.background << 8) | 0xff);
    assert_eq!(told.text, (theme.foreground << 8) | 0xff);
    assert_eq!(told.accent, (theme.primary() << 8) | 0xff);
    assert_eq!(told.ansi.len(), 16);
    assert_eq!(told.fonts.mono, "JetBrains Mono");
    assert_eq!(told.name, "dark");
}

#[test]
fn protocol_colors_are_rgba() {
    let color = theme::hsla(0xff000080);
    let rgba = gpui::Rgba::from(color);
    assert!((rgba.r - 1.).abs() < 0.01 && rgba.g < 0.01 && rgba.b < 0.01);
    assert!((rgba.a - 0.5).abs() < 0.01);
}

#[test]
fn a_modules_zoom_bounds_never_make_clamping_panic() {
    let pz = |min_zoom, max_zoom| wasm_mods_protocol::PanZoom {
        min_zoom,
        max_zoom,
        ..Default::default()
    };
    assert_eq!(canvas::zoom_range(&pz(5., 1.)), (1., 5.));
    let (lo, hi) = canvas::zoom_range(&pz(f32::NAN, -2.));
    assert!(lo > 0. && lo <= hi);
    // Every range clamps without panicking.
    for range in [pz(5., 1.), pz(0., 0.), pz(f32::INFINITY, 1.)] {
        let (lo, hi) = canvas::zoom_range(&range);
        assert!(2_f32.clamp(lo, hi).is_finite());
    }
}
