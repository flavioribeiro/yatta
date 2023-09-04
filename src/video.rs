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
        let enc = gst::ElementFactory::make("x264enc")
            .property("bframes", 0u32)
            .property("bitrate", self.bitrate as u32 / 1000u32)
            .property_from_str("tune", "zerolatency")
            .build()?;
        let parser = gst::ElementFactory::make("h264parse").build()?;
        let h264_capsfilter = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-h264")
                    .field("profile", "main")
                    .build(),
            )
            .build()?;
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
            &h264_capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        gst::Element::link_many([
            &src,
            &raw_capsfilter,
            &timeoverlay,
            &enc,
            &parser,
            &h264_capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        utils::probe_encoder(state, enc);

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }
}
