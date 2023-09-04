use gst::prelude::*;
use std::sync::{Arc, Mutex};

use crate::State;


pub(crate) fn probe_encoder(state: Arc<Mutex<State>>, enc: gst::Element) {
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
                        // TODO parse the codec_data from structure and set the mime accordingly
                        mime = "av01.0.00M.08".into();
                    }

                    let mut state = state.lock().unwrap();
                    state.all_mimes.push(mime.into());
                    state.try_write_manifest();
                    gst::PadProbeReturn::Remove
                }
                _ => gst::PadProbeReturn::Ok,
            },
            _ => gst::PadProbeReturn::Ok,
        },
    );
}
