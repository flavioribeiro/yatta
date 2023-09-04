use gst::prelude::*;
use std::sync::{Arc, Mutex};

use crate::State;


pub(crate) fn probe_encoder(state: Arc<Mutex<State>>, enc: gst::Element) {
    enc.static_pad("src").unwrap().add_probe(
        gst::PadProbeType::EVENT_DOWNSTREAM,
        move |_pad, info| match info.data {
            Some(gst::PadProbeData::Event(ref ev)) => match ev.view() {
                gst::EventView::Caps(e) => {
                    let mime = gst_pbutils::codec_utils_caps_get_mime_codec(e.caps());
                    let mut state = state.lock().unwrap();
                    state.all_mimes.push(mime.unwrap().into());
                    state.try_write_manifest();
                    gst::PadProbeReturn::Remove
                }
                _ => gst::PadProbeReturn::Ok,
            },
            _ => gst::PadProbeReturn::Ok,
        },
    );
}
