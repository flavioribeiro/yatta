use gst::prelude::*;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Error;
use chrono::TimeDelta;

use crate::{hlscmaf, utils, State};

pub(crate) struct VideoStream {
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
        let codec_burn_in = gst::ElementFactory::make("textoverlay")
            .property("text", &self.codec)
            .property("font-desc", "Sans 24")
            .build()?;
        let Ok((enc, parser, capsfilter)) = Self::setup_codec(self) else {
            return Err(anyhow::anyhow!("Failed to setup codec: {}", self.codec));
        };

        let mux = gst::ElementFactory::make("isofmp4mux")
            .property("fragment-duration", 2000.mseconds())
            .property_from_str("header-update-mode", "update")
            .property("write-mehd", true)
            .build()?;
        let appsink = gst_app::AppSink::builder().buffer_list(true).build();

        pipeline.add_many([
            &src,
            &raw_capsfilter,
            &timeoverlay,
            &codec_burn_in,
            &enc,
            &parser,
            &capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        gst::Element::link_many([
            &src,
            &raw_capsfilter,
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
        let parser: gst::Element;
        let capsfilter: gst::Element;

        let enc_factory = encoder_for_codec(&self.codec)
            .expect(&format!("No encoder found for codec: {}", self.codec));
        let enc = enc_factory.create().build()?;

        match self.codec.as_ref() {
            "h264" => {
                if enc.has_property("bitrate", None) {
                    enc.set_property("bitrate", self.bitrate as u32 / 1000u32);
                }
                if enc.has_property("realtime", None) {
                    enc.set_property("realtime", true);
                }
                // if enc.has_property("max-keyframe-interval-duration", None) {
                //     enc.set_property("max-keyframe-interval-duration", gst::ClockTime::from_seconds(1).mseconds());
                // }
                if enc_factory.name() == "x264enc" {
                    enc.set_property("bframes", 0u32);
                    enc.set_property_from_str("tune", "zerolatency");
                }
                parser = gst::ElementFactory::make("h264parse").build()?;
                capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-h264")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((enc, parser, capsfilter))
            }
            "h265" => {
                if enc.has_property("bitrate", None) {
                    enc.set_property("bitrate", self.bitrate as u32 / 1000u32);
                }
                if enc.has_property("realtime", None) {
                    enc.set_property("realtime", true);
                }
                // if enc.has_property("max-keyframe-interval-duration", None) {
                //     enc.set_property("max-keyframe-interval-duration", gst::ClockTime::from_seconds(1).mseconds());
                // }
                if enc_factory.name() == "x264enc" {
                    enc.set_property_from_str("tune", "zerolatency");
                }
                parser = gst::ElementFactory::make("h265parse").build()?;
                capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-h265")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((enc, parser, capsfilter))
            }
            "av1" => {
                if enc_factory.name() == "rav1enc" {
                    enc.set_property("speed-preset", 10u32);
                    enc.set_property("low-latency", true);
                    enc.set_property(
                        "max-key-frame-interval",
                        gst::ClockTime::from_seconds(1).mseconds(),
                    );
                    enc.set_property("bitrate", self.bitrate as i32);
                }
                parser = gst::ElementFactory::make("av1parse").build()?;
                capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-av1")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((enc, parser, capsfilter))
            }
            &_ => todo!(),
        }
    }
}

fn encoder_for_codec(codec: &String) -> Option<gst::ElementFactory> {
    let encoders = gst::ElementFactory::factories_with_type(
        gst::ElementFactoryType::ENCODER,
        gst::Rank::MARGINAL,
    );
    let caps = gst::Caps::new_empty_simple(format!("video/x-{}", codec));
    encoders
        .iter()
        .find(|factory| {
            factory.static_pad_templates().iter().any(|template| {
                let template_caps = template.caps();
                template.direction() == gst::PadDirection::Src
                    && !template_caps.is_any()
                    && caps.can_intersect(&template_caps)
            })
        })
        .cloned()
}
