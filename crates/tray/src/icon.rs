/// Direct ARGB artwork keeps the probe independent of installed icon themes.
/// The badge is composed into the main icon, without relying on host overlays.
pub fn icon(size: i32, unavailable: bool) -> ksni::Icon {
    let mut argb_data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (x, y) = (
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            let mut pixel = [0, 0, 0, 0];
            if (x - 0.5).powi(2) + (y - 0.5).powi(2) < 0.45_f32.powi(2) {
                pixel = [255, 40, 140, 230];
            }
            if (0.35..0.73).contains(&x) && (y - 0.5).abs() < (0.73 - x) * 0.65 {
                pixel = [255, 255, 255, 255];
            }
            if unavailable && (x - 0.78).powi(2) + (y - 0.22).powi(2) < 0.21_f32.powi(2) {
                pixel = [255, 220, 35, 45];
                if (0.75..0.81).contains(&x)
                    && ((0.08..0.24).contains(&y) || (0.28..0.34).contains(&y))
                {
                    pixel = [255, 255, 255, 255];
                }
            }
            argb_data.extend_from_slice(&pixel);
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data: argb_data,
    }
}
