use gst::prelude::*;
use std::{sync::{Mutex, Arc}, path::Path};

use anyhow::Error;

use crate::{State, hlscmaf, utils};

pub(crate) struct VideoStream {
    pub name: String,
    pub bitrate: u64,
    pub width: u64,
    pub height: u64,
}

impl VideoStream {
    pub fn setup(
        &self,
        state: Arc<Mutex<State>>,
        pipeline: &gst::Pipeline,
        path: &Path,
    ) -> Result<(), Error> {
        let src = gst::ElementFactory::make("videotestsrc")
            .property("is-live", true)
            .build()?;

        let raw_capsfilter = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::I420)
                    .width(self.width as i32)
                    .height(self.height as i32)
                    .framerate(30.into())
                    .build(),
            )
            .build()?;
        let timeoverlay = gst::ElementFactory::make("timeoverlay").build()?;
        let enc = gst::ElementFactory::make("rav1enc")
            .property("speed-preset", 10 as u32)
            .property("low-latency", true)
            .property("max-key-frame-interval", 60 as u64)
            .property("bitrate", self.bitrate as i32)
            .build()?;
        let parser = gst::ElementFactory::make("av1parse").build()?;

        let mux = gst::ElementFactory::make("cmafmux")
            .property("fragment-duration", 2000.mseconds())
            .property_from_str("header-update-mode", "update")
            .property("write-mehd", true)
            .build()?;
        let appsink = gst_app::AppSink::builder().buffer_list(true).build();

        pipeline.add_many([
            &src,
            &raw_capsfilter,
            &timeoverlay,
            &enc,
            &parser,
            &mux,
            appsink.upcast_ref(),
        ])?;

        gst::Element::link_many([
            &src,
            &raw_capsfilter,
            &timeoverlay,
            &enc,
            &parser,
            &mux,
            appsink.upcast_ref(),
        ])?;

        utils::probe_encoder(state, enc);

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }
}
