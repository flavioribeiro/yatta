use gst::prelude::*;
use std::{sync::{Mutex, Arc}, path::Path};

use anyhow::Error;

use crate::{State, hlscmaf, utils};

pub(crate) struct VideoStream {
    pub src: gst::Element,
    pub name: String,
    pub codec: String,
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
        let timeoverlay = gst::ElementFactory::make("timeoverlay").build()?;
        let codec_burn_in = gst::ElementFactory::make("textoverlay")
            .property("text", &self.codec)
            .property("font-desc", "Sans 24")
            .build()?;
        let Ok((enc, parser, capsfilter)) = Self::setup_codec(self) else { todo!() };

        let mux = gst::ElementFactory::make("isofmp4mux")
            .property("fragment-duration", 2000.mseconds())
            .property_from_str("header-update-mode", "update")
            .property("write-mehd", true)
            .build()?;
        let appsink = gst_app::AppSink::builder().buffer_list(true).build();

        pipeline.add_many([
            &self.src,
            &timeoverlay,
            &codec_burn_in,
            &enc,
            &parser,
            &capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        gst::Element::link_many([
            &self.src,
            &timeoverlay,
            &codec_burn_in,
            &enc,
            &parser,
            &capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        utils::probe_encoder(state, enc, self.name.clone());

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }

    fn setup_codec(&self) -> Result<(gst::Element, gst::Element, gst::Element), Error> {
        let mut _enc: gst::Element;
        let mut _parser: gst::Element;
        let mut _capsfilter: gst::Element;

        match self.codec.as_ref() {
            "h264" => {
                _enc = gst::ElementFactory::make("x264enc")
                    .property("bframes", 0u32)
                    .property("bitrate", self.bitrate as u32 / 1000u32)
                    .property_from_str("tune", "zerolatency")
                    .build()?;
                _parser = gst::ElementFactory::make("h264parse").build()?;
                _capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-h264")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((_enc, _parser, _capsfilter))
            }
            "h265" => {
                _enc = gst::ElementFactory::make("x265enc")
                    .property("bitrate", self.bitrate as u32 / 1000u32)
                    .property_from_str("tune", "zerolatency")
                    .build()?;
                _parser = gst::ElementFactory::make("h265parse").build()?;
                _capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-h265")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((_enc, _parser, _capsfilter))
            }
            "av1" => {
                _enc = gst::ElementFactory::make("rav1enc")
                .property("speed-preset", 10u32)
                .property("low-latency", true)
                .property("max-key-frame-interval", 60u64)
                .property("bitrate", self.bitrate as i32)
                .build()?;
                _parser = gst::ElementFactory::make("av1parse").build()?;
                _capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-av1")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((_enc, _parser, _capsfilter))
            }
            &_ => todo!()
        }
    }
}
