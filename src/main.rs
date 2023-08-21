use gst::prelude::*;
use log::info;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Error;
use chrono::{DateTime, Utc};
use m3u8_rs::{AlternativeMedia, AlternativeMediaType, MasterPlaylist, VariantStream};

mod hlscmaf;

struct State {
    video_streams: Vec<VideoStream>,
    audio_streams: Vec<AudioStream>,
    all_mimes: Vec<String>,
    path: PathBuf,
    wrote_manifest: bool,
}

impl State {
    fn maybe_write_manifest(&mut self) {
        if self.wrote_manifest {
            return;
        }

        if self.all_mimes.len() < self.video_streams.len() + self.audio_streams.len() {
            return;
        }

        let mut all_mimes = self.all_mimes.clone();
        all_mimes.sort();
        all_mimes.dedup();

        let playlist = MasterPlaylist {
            version: Some(7),
            variants: self
                .video_streams
                .iter()
                .map(|stream| {
                    let mut path = PathBuf::new();

                    path.push(&stream.name);
                    path.push("manifest.m3u8");

                    VariantStream {
                        uri: path.as_path().display().to_string(),
                        bandwidth: stream.bitrate,
                        codecs: Some(all_mimes.join(",")),
                        resolution: Some(m3u8_rs::Resolution {
                            width: stream.width,
                            height: stream.height,
                        }),
                        audio: Some("audio".to_string()),
                        ..Default::default()
                    }
                })
                .collect(),
            alternatives: self
                .audio_streams
                .iter()
                .map(|stream| {
                    let mut path = PathBuf::new();
                    path.push(&stream.name);
                    path.push("manifest.m3u8");

                    AlternativeMedia {
                        media_type: AlternativeMediaType::Audio,
                        uri: Some(path.as_path().display().to_string()),
                        group_id: "audio".to_string(),
                        language: Some(stream.lang.clone()),
                        name: stream.name.clone(),
                        default: stream.default,
                        autoselect: stream.default,
                        channels: Some("2".to_string()),
                        ..Default::default()
                    }
                })
                .collect(),
            independent_segments: true,
            ..Default::default()
        };

        let mut file = std::fs::File::create(&self.path).unwrap();
        playlist
            .write_to(&mut file)
            .expect("Failed to write master playlist");

        info!("wrote master manifest to {}", self.path.display());
        self.wrote_manifest = true;
    }
}

struct Segment {
    date_time: DateTime<Utc>,
    duration: gst::ClockTime,
    path: String,
}

struct UnreffedSegment {
    removal_time: DateTime<Utc>,
    path: String,
}

struct StreamState {
    path: PathBuf,
    segments: VecDeque<Segment>,
    trimmed_segments: VecDeque<UnreffedSegment>,
    start_date_time: Option<DateTime<Utc>>,
    start_time: Option<gst::ClockTime>,
    media_sequence: u64,
    segment_index: u32,
}

struct VideoStream {
    name: String,
    bitrate: u64,
    width: u64,
    height: u64,
}

struct AudioStream {
    name: String,
    lang: String,
    default: bool,
    wave: String,
}

fn probe_encoder(state: Arc<Mutex<State>>, enc: gst::Element) {
    enc.static_pad("src").unwrap().add_probe(
        gst::PadProbeType::EVENT_DOWNSTREAM,
        move |_pad, info| match info.data {
            Some(gst::PadProbeData::Event(ref ev)) => match ev.view() {
                gst::EventView::Caps(e) => {
                    let mime = gst_pbutils::codec_utils_caps_get_mime_codec(e.caps());

                    let mut state = state.lock().unwrap();
                    state.all_mimes.push(mime.unwrap().into());
                    state.maybe_write_manifest();
                    gst::PadProbeReturn::Remove
                }
                _ => gst::PadProbeReturn::Ok,
            },
            _ => gst::PadProbeReturn::Ok,
        },
    );
}

impl VideoStream {
    fn setup(
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
            &h264_capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        gst::Element::link_many([
            &src,
            &raw_capsfilter,
            &timeoverlay,
            &enc,
            &h264_capsfilter,
            &mux,
            appsink.upcast_ref(),
        ])?;

        probe_encoder(state, enc);

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }
}

impl AudioStream {
    fn setup(
        &self,
        state: Arc<Mutex<State>>,
        pipeline: &gst::Pipeline,
        path: &Path,
    ) -> Result<(), Error> {
        let src = gst::ElementFactory::make("audiotestsrc")
            .property("is-live", true)
            .property_from_str("wave", &self.wave)
            .build()?;
        let enc = gst::ElementFactory::make("avenc_aac").build()?;
        let mux = gst::ElementFactory::make("cmafmux")
            .property_from_str("header-update-mode", "update")
            .property("write-mehd", true)
            .property("fragment-duration", 2000.mseconds())
            .build()?;
        let appsink = gst_app::AppSink::builder().buffer_list(true).build();

        pipeline.add_many([&src, &enc, &mux, appsink.upcast_ref()])?;

        gst::Element::link_many([&src, &enc, &mux, appsink.upcast_ref()])?;

        probe_encoder(state, enc);

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }
}

fn main() -> Result<(), Error> {
    gst::init()?;
    env_logger::init();

    let path = PathBuf::from("hls_live_stream");
    let pipeline = gst::Pipeline::default();
    std::fs::create_dir_all(&path).expect("failed to create directory");

    let mut manifest_path = path.clone();
    manifest_path.push("manifest.m3u8");

    let state = Arc::new(Mutex::new(State {
        video_streams: vec![VideoStream {
            name: "video_0".to_string(),
            bitrate: 2_048_000,
            width: 1280,
            height: 720,
        }],
        audio_streams: vec![
            AudioStream {
                name: "audio_0".to_string(),
                lang: "eng".to_string(),
                default: true,
                wave: "sine".to_string(),
            },
            AudioStream {
                name: "audio_1".to_string(),
                lang: "fre".to_string(),
                default: false,
                wave: "white-noise".to_string(),
            },
        ],
        all_mimes: vec![],
        path: manifest_path.clone(),
        wrote_manifest: false,
    }));

    {
        let state_lock = state.lock().unwrap();

        for stream in &state_lock.video_streams {
            stream.setup(state.clone(), &pipeline, &path)?;
        }

        for stream in &state_lock.audio_streams {
            stream.setup(state.clone(), &pipeline, &path)?;
        }
    }

    pipeline.set_state(gst::State::Playing)?;

    let bus = pipeline
        .bus()
        .expect("Pipeline without bus. Shouldn't happen!");

    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;

        match msg.view() {
            MessageView::Eos(..) => {
                println!("EOS");
                break;
            }
            MessageView::Error(err) => {
                pipeline.set_state(gst::State::Null)?;
                eprintln!(
                    "Got error from {}: {} ({})",
                    msg.src()
                        .map(|s| String::from(s.path_string()))
                        .unwrap_or_else(|| "None".into()),
                    err.error(),
                    err.debug().unwrap_or_else(|| "".into()),
                );
                break;
            }
            _ => (),
        }
    }

    pipeline.set_state(gst::State::Null)?;

    Ok(())
}
