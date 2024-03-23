use std::fmt::Display;
use std::sync::{Arc, Mutex};

use anyhow::Error;
use gst::prelude::*;

use crate::{hlscmaf, utils, State};

#[derive(Debug, Clone)]
pub(crate) struct VideoStream {
    pub name: String,
    pub codec: VideoCodec,
    pub bitrate: u64,
    pub level: String,
    pub width: u64,
    pub height: u64,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum VideoCodec {
    H264,
    H265,
    AV1,
}

impl VideoCodec {
    pub fn caps(&self) -> gst::Caps {
        match self {
            VideoCodec::H264 => gst::Caps::builder("video/x-h264").build(),
            VideoCodec::H265 => gst::Caps::builder("video/x-h265").build(),
            VideoCodec::AV1 => gst::Caps::builder("video/x-av1").build(),
        }
    }
}

impl Display for VideoCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            VideoCodec::H264 => "h264".to_string(),
            VideoCodec::H265 => "h265".to_string(),
            VideoCodec::AV1 => "av1".to_string(),
        };
        write!(f, "{}", str)
    }
}

impl VideoStream {
    pub fn setup(
        &self,
        state: Arc<Mutex<State>>,
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        path: &[String],
        forced_encoder_factory_name: Option<String>,
        fragment_duration_nanos: u64,
    ) -> Result<(), Error> {
        let frame_rate = gst::Fraction::new(30, 1);

        let queue = gst::ElementFactory::make("queue")
            .name(format!("{}-queue", self.name))
            .build()?;
        let videoscale = gst::ElementFactory::make("videoscale")
            .name(format!("{}-videoscale", self.name))
            .build()?;
        let videorate = gst::ElementFactory::make("videorate")
            .name(format!("{}-videorate", self.name))
            .build()?;
        let raw_capsfilter = gst::ElementFactory::make("capsfilter")
            .name(format!("{}-video-capsfilter", self.name))
            .property(
                "caps",
                gst_video::VideoCapsBuilder::new()
                    .format(gst_video::VideoFormat::I420)
                    .width(self.width as i32)
                    .height(self.height as i32)
                    .framerate(frame_rate)
                    .build(),
            )
            .build()?;
        let codec_burn_in = gst::ElementFactory::make("textoverlay")
            .name(format!("{}-textoverlay", self.name))
            .property("text", &self.name)
            .property("font-desc", "Sans 24")
            .build()?;
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .name(format!("{}-videoconvert", self.name))
            .build()?;
        let Ok((enc, parser, capsfilter)) = self.setup_codec(
            forced_encoder_factory_name.as_deref(),
            fragment_duration_nanos,
            frame_rate,
        ) else {
            return Err(anyhow::anyhow!("Failed to setup codec: {}", self.name));
        };

        let mux = {
            // if self.codec == VideoCodec::AV1 {
            //     gst::ElementFactory::make("isofmp4mux").name(format!("{}-isofmp4mux", self.name))
            // } else {
            //     gst::ElementFactory::make("cmafmux").name(format!("{}-cmafmux", self.name))
            // }
            gst::ElementFactory::make("cmafmux")
                .name(format!("{}-cmafmux", self.name))
                .property("fragment-duration", fragment_duration_nanos)
                .property("latency", gst::ClockTime::from_seconds(1).nseconds())
                .property_from_str("header-update-mode", "update")
                .property("write-mehd", true)
                .build()?
        };
        let appsink = gst_app::AppSink::builder()
            .name(format!("{}-appsink", self.name))
            .buffer_list(true)
            .build();

        pipeline.add_many([
            &queue,
            &videoscale,
            &videorate,
            &raw_capsfilter,
            &codec_burn_in,
            &videoconvert,
            &enc,
            &parser,
            &capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        src_pad
            .link(&queue.static_pad("sink").unwrap())
            .expect("Failed to link video_src_pad to queue");
        gst::Element::link_many([
            &queue,
            &videoscale,
            &videorate,
            &raw_capsfilter,
            &codec_burn_in,
            &videoconvert,
            &enc,
            &parser,
            &capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        utils::probe_encoder(state, parser, self.name.clone());

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }

    fn setup_codec(
        &self,
        forced_encoder_factory_name: Option<&str>,
        fragment_duration_nanos: u64,
        frame_rate: gst::Fraction,
    ) -> Result<(gst::Element, gst::Element, gst::Element), Error> {
        let parser: gst::Element;
        let capsfilter: gst::Element;

        let frames_per_fragment: u64 = gst::ClockTime::from_nseconds(fragment_duration_nanos)
            .seconds()
            .mul_div_ceil(frame_rate.numer() as u64, frame_rate.denom() as u64)
            .unwrap();

        let enc_factory = match forced_encoder_factory_name {
            Some(enc) => gst::ElementFactory::find(enc)
                .expect(&format!("No encoder with this name was found: {}", enc)),
            None => encoder_for_codec(self.codec)
                .expect(&format!("No encoder found for codec: {}", self.codec)),
        };
        let enc = enc_factory.create().build()?;

        match self.codec {
            VideoCodec::H264 => {
                if enc.has_property("bitrate", None) {
                    enc.set_property("bitrate", self.bitrate as u32 / 1000u32);
                }
                if enc.has_property("realtime", None) {
                    enc.set_property("realtime", true);
                }
                if enc_factory.name() == "x264enc" {
                    enc.set_property("bframes", 0u32);
                    enc.set_property_from_str("tune", "zerolatency");
                    enc.set_property("key-int-max", frames_per_fragment as u32);
                }
                if enc.has_property("max-keyframe-interval-duration", None) {
                    enc.set_property("max-keyframe-interval-duration", fragment_duration_nanos);
                }
                if enc.has_property("xcoder-params", None) {
                    enc.set_property(
                        "xcoder-params",
                        format!(
                            "RcEnable=1:gopPresetIdx=9:bitrate={}:intraPeriod={}",
                            self.bitrate, frames_per_fragment
                        ),
                    );
                }
                parser = gst::ElementFactory::make("h264parse")
                    .name(format!("{}-h264parse", self.name))
                    .build()?;
                capsfilter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-h264")
                            .field("profile", "high")
                            .build(),
                    )
                    .build()?;
                Ok((enc, parser, capsfilter))
            }
            VideoCodec::H265 => {
                if enc.has_property("bitrate", None) {
                    enc.set_property("bitrate", self.bitrate as u32 / 1000u32);
                }
                if enc.has_property("realtime", None) {
                    enc.set_property("realtime", true);
                }
                if enc.has_property("xcoder-params", None) {
                    enc.set_property(
                        "xcoder-params",
                        format!(
                            "RcEnable=1:gopPresetIdx=9:bitrate={}:intraPeriod={}",
                            self.bitrate, frames_per_fragment
                        ),
                    );
                }
                parser = gst::ElementFactory::make("h265parse")
                    .name(format!("{}-h265parse", self.name))
                    .build()?;
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
            VideoCodec::AV1 => {
                if enc_factory.name() == "rav1enc" {
                    enc.set_property("speed-preset", 10u32);
                    enc.set_property("low-latency", true);
                    enc.set_property("error-resilient", true);
                    enc.set_property("threads", 10u32);
                    enc.set_property("rdo-lookahead-frames", 2i32);
                    enc.set_property("max-key-frame-interval", frames_per_fragment);
                    enc.set_property("bitrate", self.bitrate as i32 / 1000i32);
                }
                if enc_factory.name() == "av1enc" {
                    enc.set_property("cpu-used", 10i32);
                    // enc.set_property_from_str("end-usage", "cbr");
                    enc.set_property_from_str("usage-profile", "realtime");
                    enc.set_property("threads", 50u32);
                    enc.set_property("keyframe-max-dist", frames_per_fragment as i32);
                    enc.set_property("target-bitrate", self.bitrate as u32);
                }
                if enc_factory.name() == "nvav1enc" {
                    enc.set_property("bitrate", self.bitrate as u32);
                    enc.set_property("gop-length", frames_per_fragment as u32);
                }
                if enc.has_property("xcoder-params", None) {
                    enc.set_property(
                        "xcoder-params",
                        format!(
                            "roiEnable=1:profile=1:high-tier=0:lowDelay=1:lookaheadDepth=0:multicoreJointMode=0:gopPresetIdx=9:av1ErrorResilientMode=1:RcEnable=1:bitrate={}:intraPeriod={}",
                            self.bitrate,
                            frames_per_fragment
                        ),
                    );
                }
                parser = gst::ElementFactory::make("av1parse") // av1parse
                    .name(format!("{}-av1parse", self.name))
                    .build()?;
                capsfilter = gst::ElementFactory::make("capsfilter")
                    .name(format!("{}-capsfilter", self.name))
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-av1")
                            .field("profile", "main")
                            .build(),
                    )
                    .build()?;
                Ok((enc, parser, capsfilter))
            }
        }
    }

    pub fn manifest_path(&self) -> String {
        format!("{}.m3u8", self.name)
    }
}

fn encoder_for_codec(codec: VideoCodec) -> Option<gst::ElementFactory> {
    let encoders =
        gst::ElementFactory::factories_with_type(gst::ElementFactoryType::ENCODER, gst::Rank::NONE)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
    let caps = codec.caps();
    // sort encoders if name starts with niquadra
    let encoders = if codec != VideoCodec::AV1 {
        encoders
            .iter()
            .filter(|factory| factory.name().starts_with("niquadra"))
            .chain(
                encoders
                    .iter()
                    .filter(|factory| !factory.name().starts_with("niquadra")),
            )
            .cloned()
            .collect::<Vec<_>>()
    } else {
        encoders
    };
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
