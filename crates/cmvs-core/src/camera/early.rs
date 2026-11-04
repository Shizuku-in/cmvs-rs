//! cmvs-20071117 x86: 0x410c10 -> 0x42df60, types 1 and 2.
use super::{Camera, ObjectProjection};

pub(super) fn reset_defaults(camera: &mut Camera) {
    // 0x42dc10: the early constructor's height is 614, with center Y = 307.
    camera.words[10] = 614.0f32.to_bits();
    camera.words[16] = 307.0f32.to_bits();
}

pub(super) fn supported(profile: &crate::profile::EngineProfile) -> bool {
    profile.id == "cmvs-20071117"
        && profile
            .binary(
                &crate::digest::parse_hex(
                    "caff7901a7dc4ec963d7fde358028cdd68dfd87fc7925c9715a20e7a5f36d02b",
                )
                .expect("static SHA-256"),
            )
            .is_some()
}

pub(super) fn project(
    camera: &Camera,
    [x, y, z]: [f32; 3],
    offset: f32,
    reference: f32,
) -> Result<Option<ObjectProjection>, &'static str> {
    if camera.words[0] == 2 {
        return Ok(project_type_two(camera, [x, y, z], offset, reference));
    }
    if camera.words[0] != 1 {
        return Err("early camera projection types other than 1 and 2 are not verified");
    }
    let depth = z - camera.float(3);
    if depth.is_nan() || depth <= 0.0 {
        return Ok(None);
    }
    // Keep the x87 expression order and explicit single-precision spills.
    // f64 intermediates model this branch; no claim of full x87 bit equivalence.
    let delta = ((f64::from(reference) - f64::from(depth)) * 0.5) as f32;
    let scale = if reference == depth {
        1.0
    } else if delta < 0.0 {
        let denominator = delta.abs() + 1.0;
        (1.0 / f64::from(denominator)) as f32
    } else {
        delta + 1.0
    };
    let fade = if depth >= 0.5 {
        255
    } else {
        crate::evaluate::truncate(depth * 512.0)
    };
    let f = |i| f64::from(camera.float(i));
    let half = (f(6) * 0.5) as f32;
    let gap = ((f(6) - f64::from(depth)) * 0.5) as f32;
    let perspective = 1.0 / ((f64::from(half) - f64::from(gap)) / f64::from(half));
    let horizontal = ((f64::from(x) - f(1)) * perspective) as f32;
    let horizontal_scale = (f(9) / f(7)) as f32;
    let screen_x = (f64::from(horizontal) * f64::from(horizontal_scale) + f(9) * 0.5) as f32;
    let vertical = ((f(2) - f64::from(y)) * perspective) as f32;
    let vertical_scale = (f(10) / f(8)) as f32;
    let screen_y = ((f64::from(vertical) + f64::from(offset)) * f64::from(vertical_scale)
        + f(10) * 0.5) as f32;
    let screen_y = (f64::from(screen_y) + (f64::from(reference) - f64::from(depth)) * f(11)) as f32;
    Ok(Some(ObjectProjection {
        position: [screen_x, screen_y],
        depth,
        scale,
        fade,
    }))
}

fn project_type_two(
    camera: &Camera,
    [x, y, z]: [f32; 3],
    offset: f32,
    reference: f32,
) -> Option<ObjectProjection> {
    let depth = z - camera.float(3);
    if depth.is_nan() || depth <= 9.0 {
        return None;
    }
    let f = |i| f64::from(camera.float(i));
    let shift = if camera.float(13) == 0.0 {
        0.0
    } else {
        let half = (f(7) * 0.5) as f32;
        (f64::from(half) * (f(13) / 45.0) * (f64::from(depth) / f(6))) as f32
    };
    let ratio = (f64::from(reference) / f64::from(depth)) as f32;
    let scale = (f64::from(ratio) * f64::from(ratio)) as f32;
    let relative = (f64::from(depth) / f(6)) as f32;
    let horizontal_scale = (f(9) / (f64::from(relative) * f(7))) as f32;
    let horizontal = ((f64::from(x) - (f(1) + f64::from(shift))) * f64::from(ratio)) as f32;
    let screen_x = (f64::from(horizontal_scale) * f64::from(horizontal) + f(15)) as f32;
    let vertical = (f64::from(ratio) * (f(2) - f64::from(y))) as f32;
    let vertical_scale = (f(10) / f(8)) as f32;
    let screen_y =
        ((f64::from(vertical) + f64::from(offset)) * f64::from(vertical_scale) + f(16)) as f32;
    let screen_y = (f64::from(screen_y)
        + (f64::from(reference) - f64::from(depth) + f64::from(0.3f32)) * f(11))
        as f32;
    Some(ObjectProjection {
        position: [screen_x, screen_y],
        depth,
        scale,
        fade: if depth >= 10.0 {
            255
        } else {
            crate::evaluate::truncate((depth - 9.0) * 256.0)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_defaults_survive_reset_and_type_two_uses_the_horizontal_shift() {
        let profile = crate::profile::Registry::builtin()
            .profile("cmvs-20071117")
            .unwrap();
        let mut host = super::super::CameraHost::for_profile(profile);
        assert_eq!(host.camera(0).unwrap().float(10), 614.0);
        host.cameras[0].words[10] = 1.0f32.to_bits();
        host.reset();
        assert_eq!(host.camera(0).unwrap().float(10), 614.0);
        assert_eq!(host.camera(0).unwrap().float(16), 307.0);
        let mut camera = Camera::default();
        camera.words[0] = 2;
        for (i, value) in [
            (2, 0.0f32),
            (7, 8.0),
            (8, 4.0),
            (9, 8.0),
            (10, 4.0),
            (11, 0.0),
            (13, 45.0),
            (15, 4.0),
            (16, 2.0),
        ] {
            camera.words[i] = value.to_bits();
        }
        assert_eq!(
            project(&camera, [2.0, 1.0, 20.0], 0.5, 20.0)
                .unwrap()
                .unwrap()
                .position,
            [1.0, 1.5]
        );
        assert_eq!(
            super::super::CameraHost::default()
                .camera(0)
                .unwrap()
                .float(10),
            612.0
        );
    }
}
