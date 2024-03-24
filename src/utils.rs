use gst::prelude::*;
use std::sync::{Arc, Mutex};

use crate::State;

pub(crate) fn probe_encoder(state: Arc<Mutex<State>>, enc: gst::Element, name: String) {
    enc.static_pad("src").unwrap().add_probe(
        gst::PadProbeType::EVENT_DOWNSTREAM,
        move |_pad, info| match info.data {
            Some(gst::PadProbeData::Event(ref ev)) => match ev.view() {
                gst::EventView::Caps(e) => {
                    let mut mime = gst_pbutils::codec_utils_caps_get_mime_codec(e.caps()).unwrap();
                    let structure = e.caps().structure(0).unwrap();
                    if structure.name() == "video/x-av1" {
                        // https://www.reddit.com/r/AV1/comments/stbk3r/av1_in_hls_manifests_works_for_browsers_that/
                        // https://github.com/GStreamer/gst-plugins-base/blob/master/gst-libs/gst/pbutils/codec-utils.c#L2402-L2404
                        // https://aomediacodec.github.io/av1-isobmff/#codecsparam
                        if let Ok(codec_data) = structure.get::<&gst::BufferRef>("codec_data") {
                            let map = codec_data.map_readable().unwrap();
                            mime = compute_av1_mime(
                                map.as_slice(),
                                structure
                                    .get::<&str>("colorimetry")
                                    .ok()
                                    .and_then(|c| c.parse::<gst_video::VideoColorimetry>().ok()),
                            )
                            .into();
                            log::debug!("Computed mime for {}: {}", name, mime);
                        } else {
                            log::debug!("No codec_data found for AV1 encoder, using default mime");
                            // Fallback to a default mime
                            mime = "av01.0.00M.08".into();
                        }
                    }
                    let mut state = state.lock().unwrap();
                    state.all_mimes.insert(name.to_string(), mime.into());
                    state.try_write_manifest();
                    gst::PadProbeReturn::Remove
                }
                _ => gst::PadProbeReturn::Ok,
            },
            _ => gst::PadProbeReturn::Ok,
        },
    );
}

// Parse the AV1CodecConfigurationRecord from the codec_data buffer and calculate the mime type.
//
// Syntax of data is:
// aligned(8) class AV1CodecConfigurationRecord {
//
//     unsigned int(1) marker = 1;
//     unsigned int(7) version = 1;
//
//     unsigned int(3) seq_profile;
//     unsigned int(5) seq_level_idx_0;
//
//     unsigned int(1) seq_tier_0;
//     unsigned int(1) high_bitdepth;
//     unsigned int(1) twelve_bit;
//     unsigned int(1) monochrome;
//     unsigned int(1) chroma_subsampling_x;
//     unsigned int(1) chroma_subsampling_y;
//     unsigned int(2) chroma_sample_position;
//
//     unsigned int(3) reserved = 0;
//     ...
//  }
// https://aomediacodec.github.io/av1-isobmff/#av1codecconfigurationbox-syntax
//
// The codecs parameter string for the AOM AV1 codec is as follows:
//
// <sample entry 4CC>.<profile>.<level><tier>.<bitDepth>
//
// All fields following the sample entry 4CC are expressed as double digit decimals, unless indicated
// otherwise. Leading or trailing zeros cannot be omitted.
//
// The profile parameter value, represented by a single digit decimal, SHALL equal the value of
// seq_profile in the Sequence Header OBU. The level parameter value SHALL equal the first level
// value indicated by seq_level_idx in the Sequence Header OBU. The tier parameter value SHALL be equal
// to M when the first seq_tier value in the Sequence Header OBU is equal to 0, and H when it is equal
// to 1. The bitDepth parameter value SHALL equal the value of BitDepth variable as defined in [AV1]
// derived from the Sequence Header OBU.
//
// The parameters sample entry 4CC, profile, level, tier, and bitDepth are all mandatory fields.
// https://aomediacodec.github.io/av1-isobmff/#codecsparam
//
// https://aomediacodec.github.io/av1-spec/av1-spec.pdf - 5.5.2. Color config syntax
fn compute_av1_mime(codec_data: &[u8], colorimetry: Option<gst_video::VideoColorimetry>) -> String {
    assert!(codec_data.len() >= 3);
    let seq_profile = (codec_data[1] >> 5) & 0b0111;
    let seq_level_idx_0 = codec_data[1] & 0b0001_1111;
    println!(
        "seq_level_idx_0: {:08b} = {}",
        codec_data[1], seq_level_idx_0
    );
    let tier = {
        let seq_tier_0 = codec_data[2] >> 7;
        if seq_tier_0 == 0 {
            "M"
        } else {
            "H"
        }
    };
    let high_bitdepth = (codec_data[2] >> 6) & 0x01;
    let twelve_bit = (codec_data[2] >> 5) & 0x01;
    let bit_depth: u8 = match (seq_profile, high_bitdepth, twelve_bit) {
        (2, 1, 1) => 12,
        (2, 1, _) => 10,
        (_, 1, _) => 10,
        _ => 8,
    };
    let monochrome = (codec_data[2] >> 4) & 0x01;
    let chroma_subsampling_x = (codec_data[2] >> 3) & 0x01;
    let chroma_subsampling_y = (codec_data[2] >> 2) & 0x01;
    let chroma_sample_position = if chroma_subsampling_x == 1 && chroma_subsampling_y == 1 {
        codec_data[2] & 0b011
    } else {
        0
    };

    if let Some(colorimetry_info) = colorimetry {
        let (primaries, transfer, matrix) = {
            (
                (colorimetry_info.primaries().to_iso() as u16),
                (colorimetry_info.transfer().to_iso() as u16),
                (colorimetry_info.matrix().to_iso() as u16),
            )
        };

        let full_range: u8 = match colorimetry_info.range() {
            gst_video::VideoColorRange::Range0_255 => 1,
            _ => 0,
        };
        format!(
            "av01.{}.{:02}{}.{:02}.{}.{}{}{}.{:02}.{:02}.{:02}.{}",
            seq_profile,
            seq_level_idx_0,
            tier,
            bit_depth,
            monochrome,
            chroma_subsampling_x,
            chroma_subsampling_y,
            chroma_sample_position,
            primaries,
            transfer,
            matrix,
            full_range
        )
    } else {
        format!(
            "av01.{}.{:02}{}.{:02}",
            seq_profile, seq_level_idx_0, tier, bit_depth
        )
    }
    .replace(".0.110.01.01.01.0", "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    fn gst_init() {
        const INIT: Once = Once::new();
        INIT.call_once(|| {
            gst::init().unwrap();
        });
    }

    #[test]
    fn can_compute_simple_av1_mimes() {
        gst_init();

        assert_eq!(
            compute_av1_mime(&[0b1000_0001, 0b0000_0000, 0b0000_1100], None),
            "av01.0.00M.08"
        );
    }

    #[test]
    fn test_compute_full_av1_mime_with_colorimetry_with_default_values() {
        gst_init();

        let colorimetry = gst_video::VideoColorimetry::new(
            gst_video::VideoColorRange::Range16_235,
            gst_video::VideoColorMatrix::Bt709,
            gst_video::VideoTransferFunction::Bt709,
            gst_video::VideoColorPrimaries::Bt709,
        );

        assert_eq!(
            compute_av1_mime(&[0b1000_0001, 0b0000_0000, 0b0000_1100], Some(colorimetry)),
            "av01.0.00M.08"
        );
    }

    #[test]
    fn test_compute_full_av1_mime_with_specific_colorimetry() {
        gst_init();

        let colorimetry = gst_video::VideoColorimetry::new(
            gst_video::VideoColorRange::Range16_235,     // Limited range
            gst_video::VideoColorMatrix::Bt2020,         // ITU-R BT.2100 YCbCr color matrix
            gst_video::VideoTransferFunction::Smpte2084, // ITU-R BT.2100 PQ transfer characteristics
            gst_video::VideoColorPrimaries::Bt2020,      // ITU-R BT.2100 color primaries
        );

        assert_eq!(
            compute_av1_mime(&[0b1000_0001, 0b0000_0100, 0b0110_1110], Some(colorimetry)),
            "av01.0.04M.10.0.112.09.16.09.0"
        );
    }
}
