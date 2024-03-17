use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::{process, thread};

use crate::video::VideoCodec;
use anyhow::Error;
use clap::Parser;
use gst::prelude::*;
use log::{info, warn};
use m3u8_rs::{AlternativeMedia, AlternativeMediaType, MasterPlaylist, VariantStream};
use rand::random;
use tokio::runtime::Builder;

mod audio;
mod hlscmaf;
mod server;
mod utils;
mod video;

/// Yatta live encoder
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArguments {
    #[clap(required = true)]
    uri: String,

    /// Force the GStreamer AV1 encoder element to be used
    #[clap(long, short)]
    force_av1_encoder: Option<String>,

    #[clap(long)]
    disable_av1: bool,

    #[clap(long)]
    disable_h265: bool,

    #[clap(long)]
    disable_h264: bool,
}

struct State {
    video_streams: Vec<video::VideoStream>,
    audio_streams: Vec<audio::AudioStream>,
    all_mimes: HashMap<String, String>,
    path: PathBuf,
    wrote_manifest: bool,
}

impl State {
    fn try_write_manifest(&mut self) {
        if self.wrote_manifest
            || self.all_mimes.len() < self.video_streams.len() + self.audio_streams.len()
        {
            return;
        };
        self.write_manifest()
    }

    fn write_manifest(&mut self) {
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
                        codecs: self.all_mimes.get(&stream.name).map(|s| s.to_string()),
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

fn hw_encoders_init() {
    match process::Command::new("init_rsrc").output() {
        Ok(output) => {
            if output.status.success() {
                info!("init_rsrc executed successfully");
            } else {
                warn!(
                    "init_rsrc failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
        Err(e) => {
            warn!("failed to execute init_rsrc: {}", e);
        }
    }
}

fn main() -> Result<(), Error> {
    hw_encoders_init();
    gst::init()?;
    env_logger::init();

    let path = vec!["hls_live_stream".to_string()];
    let pipeline = gst::Pipeline::default();

    std::fs::create_dir_all(path.join("/").to_string()).expect("failed to create directory");

    let args = CliArguments::parse();

    let mut video_streams = Vec::new();
    if !args.disable_av1 {
        video_streams.push(video::VideoStream {
            name: "av1_0".to_string(),
            codec: VideoCodec::AV1,
            bitrate: 1_024_000,
            level: "".to_string(),
            width: 256,
            height: 144,
        });
    }
    if !args.disable_h265 {
        video_streams.push(video::VideoStream {
            name: "h265_1".to_string(),
            codec: VideoCodec::H265,
            bitrate: 8_000_000,
            level: "5.0".to_string(),
            width: 3840,
            height: 2160,
        });
        video_streams.push(video::VideoStream {
            name: "h265_2".to_string(),
            codec: VideoCodec::H265,
            bitrate: 4_000_000,
            level: "5.0".to_string(),
            width: 3840,
            height: 2160,
        });
        video_streams.push(video::VideoStream {
            name: "h265_3".to_string(),
            codec: VideoCodec::H265,
            bitrate: 2_000_000,
            level: "5.0".to_string(),
            width: 2560,
            height: 1440,
        });
        video_streams.push(video::VideoStream {
            name: "h265_4".to_string(),
            codec: VideoCodec::H265,
            bitrate: 3_000_000,
            level: "4.0".to_string(),
            width: 1920,
            height: 1080,
        });
        video_streams.push(video::VideoStream {
            name: "h265_5".to_string(),
            codec: VideoCodec::H265,
            bitrate: 1_000_000,
            level: "4.0".to_string(),
            width: 1920,
            height: 1080,
        });
        video_streams.push(video::VideoStream {
            name: "h265_6".to_string(),
            codec: VideoCodec::H265,
            bitrate: 2_000_000,
            level: "3.1".to_string(),
            width: 1280,
            height: 720,
        });
        video_streams.push(video::VideoStream {
            name: "h265_7".to_string(),
            codec: VideoCodec::H265,
            bitrate: 750_000,
            level: "3.1".to_string(),
            width: 1280,
            height: 720,
        });
        video_streams.push(video::VideoStream {
            name: "h265_8".to_string(),
            codec: VideoCodec::H265,
            bitrate: 450_000,
            level: "3.0".to_string(),
            width: 960,
            height: 540,
        });
        video_streams.push(video::VideoStream {
            name: "h265_9".to_string(),
            codec: VideoCodec::H265,
            bitrate: 300_000,
            level: "3.0".to_string(),
            width: 640,
            height: 360,
        });
        video_streams.push(video::VideoStream {
            name: "h265_10".to_string(),
            codec: VideoCodec::H265,
            bitrate: 200_000,
            level: "3.0".to_string(),
            width: 640,
            height: 360,
        });
    }
    if !args.disable_h264 {
        video_streams.push(video::VideoStream {
            name: "h264_1".to_string(),
            codec: VideoCodec::H264,
            bitrate: 6_000_000,
            level: "4.0".to_string(),
            width: 1920,
            height: 1080,
        });
        video_streams.push(video::VideoStream {
            name: "h264_2".to_string(),
            codec: VideoCodec::H264,
            bitrate: 3_000_000,
            level: "4.0".to_string(),
            width: 1920,
            height: 1080,
        });
        video_streams.push(video::VideoStream {
            name: "h264_3".to_string(),
            codec: VideoCodec::H264,
            bitrate: 3_000_000,
            level: "3.1".to_string(),
            width: 1280,
            height: 720,
        });
        video_streams.push(video::VideoStream {
            name: "h264_4".to_string(),
            codec: VideoCodec::H264,
            bitrate: 1_500_000,
            level: "3.1".to_string(),
            width: 1280,
            height: 720,
        });
        video_streams.push(video::VideoStream {
            name: "h264_5".to_string(),
            codec: VideoCodec::H264,
            bitrate: 1_500_000,
            level: "3.1".to_string(),
            width: 960,
            height: 540,
        });
        video_streams.push(video::VideoStream {
            name: "h264_6".to_string(),
            codec: VideoCodec::H264,
            bitrate: 750_000,
            level: "3.1".to_string(),
            width: 960,
            height: 540,
        });
        video_streams.push(video::VideoStream {
            name: "h264_7".to_string(),
            codec: VideoCodec::H264,
            bitrate: 450_000,
            level: "3.0".to_string(),
            width: 640,
            height: 360,
        });
        video_streams.push(video::VideoStream {
            name: "h264_8".to_string(),
            codec: VideoCodec::H264,
            bitrate: 300_000,
            level: "3.0".to_string(),
            width: 640,
            height: 360,
        });
        video_streams.push(video::VideoStream {
            name: "h264_9".to_string(),
            codec: VideoCodec::H264,
            bitrate: 200_000,
            level: "3.0".to_string(),
            width: 640,
            height: 360,
        });
    }

    let manifest_path = {
        let mut manifest_path = PathBuf::from(path.join("/").to_string());
        manifest_path.push("manifest.m3u8".to_string());
        manifest_path
    };

    let state = Arc::new(Mutex::new(State {
        video_streams,
        audio_streams: vec![audio::AudioStream {
            name: "audio_0".to_string(),
            lang: "en".to_string(),
            default: true,
        }],
        all_mimes: HashMap::new(),
        path: manifest_path,
        wrote_manifest: false,
    }));

    {
        let state_lock = state.lock().unwrap();

        let uridecodebin = gst::ElementFactory::make("uridecodebin")
            .name("contentsrc")
            .property("uri", &args.uri)
            .build()
            .unwrap();

        let video_head = gst::ElementFactory::make("videoconvert")
            .name("video-head")
            .build()
            .unwrap();
        let video_scale = gst::ElementFactory::make("videoscale").build().unwrap();
        let video_rate = gst::ElementFactory::make("videorate").build().unwrap();
        let timecode = gst::ElementFactory::make("timecodestamper")
            .property_from_str("set", "keep")
            .build()
            .unwrap();
        let timeoverlay = gst::ElementFactory::make("timeoverlay")
            .property_from_str("time-mode", "time-code")
            .build()
            .unwrap();
        let video_tee = gst::ElementFactory::make("tee")
            .name("video_tee")
            .build()
            .unwrap();

        let audio_head = gst::ElementFactory::make("capsfilter")
            .name("audio-head")
            .property("caps", &gst_audio::AudioCapsBuilder::new().build())
            .build()
            .unwrap();
        let audio_conv = gst::ElementFactory::make("audioconvert").build().unwrap();
        let audio_tee = gst::ElementFactory::make("tee")
            .name("audio_tee")
            .build()
            .unwrap();

        pipeline
            .add_many([
                &uridecodebin,
                &video_head,
                &video_scale,
                &video_rate,
                &timecode,
                &timeoverlay,
                &video_tee,
                &audio_head,
                &audio_conv,
                &audio_tee,
            ])
            .unwrap();
        gst::Element::link_many(&[
            &video_head,
            &video_scale,
            &video_rate,
            &timecode,
            &timeoverlay,
            &video_tee,
        ])
        .unwrap();
        gst::Element::link_many(&[&audio_head, &audio_conv, &audio_tee]).unwrap();

        uridecodebin.connect_pad_added({
            let video_weak = video_head.downgrade();
            let audio_weak = audio_head.downgrade();
            move |_, pad| {
                // check if pad is video
                let Some(video_elem) = video_weak.upgrade() else {
                    return;
                };
                let Some(audio_elem) = audio_weak.upgrade() else {
                    return;
                };
                let pad_caps = pad.current_caps().unwrap().to_string();
                info!("pad added with caps: {}", pad_caps);
                if pad_caps.starts_with("video/") {
                    let sinkpad = video_elem.static_pad("sink").unwrap();
                    if sinkpad.is_linked() {
                        warn!("video filter sinkpad already linked");
                        return;
                    }
                    info!("Linking video filter to {}", pad.name());
                    pad.link(&sinkpad).unwrap();
                    info!("Linked video filter to {}", pad.name());
                }

                if pad_caps.starts_with("audio/") {
                    let sinkpad = audio_elem.static_pad("sink").unwrap();
                    if sinkpad.is_linked() {
                        warn!("audio filter sinkpad already linked");
                        return;
                    }
                    info!("Linking audio filter to {}", pad.name());
                    pad.link(&sinkpad).unwrap();
                    info!("Linked audio filter to {}", pad.name());
                }
            }
        });

        for stream in &state_lock.video_streams {
            let force_encoder_factory_name =
                if stream.codec == VideoCodec::AV1 && args.force_av1_encoder.is_some() {
                    args.force_av1_encoder.clone()
                } else {
                    None
                };
            stream.setup(
                state.clone(),
                &pipeline,
                &video_tee.request_pad_simple("src_%u").unwrap(),
                &path,
                force_encoder_factory_name,
            )?;
        }

        for stream in &state_lock.audio_streams {
            stream.setup(
                state.clone(),
                &pipeline,
                &audio_tee.request_pad_simple("src_%u").unwrap(),
                &path,
            )?;
        }
    }

    pipeline.set_state(gst::State::Playing)?;

    let bus = pipeline
        .bus()
        .expect("Pipeline without bus. Shouldn't happen!");

    ctrlc::set_handler({
        let pipeline_weak = pipeline.downgrade();
        move || {
            let pipeline = pipeline_weak.upgrade().unwrap();
            pipeline.set_state(gst::State::Null).unwrap();
            process::exit(0);
        }
    })?;

    thread::spawn({
        let pipeline_weak = pipeline.downgrade();
        move || {
            let runtime = Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("http-server")
                .enable_all()
                .build()
                .unwrap();
            info!("Starting server");
            runtime.block_on(server::run(8080, pipeline_weak));
            info!("Server stopped");
        }
    });

    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;

        match msg.view() {
            MessageView::Eos(..) => {
                println!("EOS");
                break;
            }
            MessageView::Error(err) => {
                eprintln!(
                    "Got error from {}: {} ({})",
                    msg.src()
                        .map(|s| String::from(s.path_string()))
                        .unwrap_or_else(|| "None".into()),
                    err.error(),
                    err.debug().unwrap_or_else(|| "".into()),
                );
                let error_graph = server::dot_graph(&pipeline);
                let mut dot_cmd = process::Command::new("dot")
                    .arg("-Tpng")
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::piped())
                    .spawn()
                    .expect("Failed to start dot command");
                dot_cmd
                    .stdin
                    .as_mut()
                    .expect("Failed to open stdin")
                    .write_all(error_graph.as_bytes())
                    .expect("Failed to write to dot command");
                let res = dot_cmd
                    .wait_with_output()
                    .expect("Failed to wait for dot command");
                if res.status.success() {
                    let error_file = format!("error_graph_{}.png", random::<u32>());
                    std::fs::write(error_file, res.stdout).expect("Failed to write image");
                }
                pipeline.set_state(gst::State::Null)?;
                break;
            }
            _ => (),
        }
    }

    pipeline.set_state(gst::State::Null)?;

    Ok(())
}
