use std::{process, thread};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Error;
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

fn main() -> Result<(), Error> {
    gst::init()?;
    env_logger::init();

    let path = PathBuf::from("hls_live_stream");
    let pipeline = gst::Pipeline::default();
    std::fs::create_dir_all(&path).expect("failed to create directory");

    let mut manifest_path = path.clone();
    manifest_path.push("manifest.m3u8");

    let state = Arc::new(Mutex::new(State {
        video_streams: vec![
            video::VideoStream {
                name: "av1_0".to_string(),
                codec: "av1".to_string(),
                bitrate: 1_024_000,
                width: 256,
                height: 144,
            },
            video::VideoStream {
                name: "h265_0".to_string(),
                codec: "h265".to_string(),
                bitrate: 1_024_000,
                width: 640,
                height: 360,
            },
            video::VideoStream {
                name: "h264_0".to_string(),
                codec: "h264".to_string(),
                bitrate: 1_024_000,
                width: 640,
                height: 360,
            },
        ],
        audio_streams: vec![audio::AudioStream {
            name: "audio_0".to_string(),
            lang: "en".to_string(),
            default: true,
            wave: "sine".to_string(),
        }],
        all_mimes: HashMap::new(),
        path: manifest_path.clone(),
        wrote_manifest: false,
    }));

    // get the uri from the CLI arguments
    let uri = std::env::args().nth(1).expect("Usage: yatta <uri>");

    {
        let state_lock = state.lock().unwrap();

        let media_src = gst::parse::bin_from_description(
            "videoconvert name=vconv ! videoscale ! videorate ! timeoverlay ! video/x-raw ! queue ! tee name=video_tee",
            false,
        )?;

        let uridecodebin = gst::ElementFactory::make("uridecodebin")
            .name("contentsrc")
            .property("uri", &uri)
            .build()?;

        let video_caps_filter = media_src
            .by_name("vconv")
            .expect("vconv element must exist");

        let audio_caps_filter = gst::ElementFactory::make("capsfilter")
            .name("capsfilter")
            .property("caps", &gst_audio::AudioCapsBuilder::new().build())
            .build()
            .unwrap();
        let audio_conv = gst::ElementFactory::make("audioconvert").build().unwrap();
        let audio_tee = gst::ElementFactory::make("tee")
            .name("audio_tee")
            .build()
            .unwrap();

        media_src
            .add_many([&uridecodebin, &audio_caps_filter, &audio_conv, &audio_tee])
            .unwrap();
        gst::Element::link_many(&[&audio_caps_filter, &audio_conv, &audio_tee]).unwrap();

        uridecodebin.connect_pad_added({
            let video_weak = video_caps_filter.downgrade();
            let audio_weak = audio_caps_filter.downgrade();
            move |elem, pad| {
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

        pipeline
            .add(&media_src)
            .expect("Failed to add video_src to pipeline");
        let video_tee = media_src
            .by_name("video_tee")
            .expect("tee element must exist");

        for stream in &state_lock.video_streams {
            // request pad from tee for each stream
            let video_src_pad =
                gst::GhostPad::with_target(&video_tee.request_pad_simple("src_%u").unwrap())?;
            media_src.add_pad(&video_src_pad).unwrap();
            stream.setup(state.clone(), &pipeline, video_src_pad.upcast_ref(), &path)?;
        }

        for stream in &state_lock.audio_streams {
            // set different name for ghost pad
            let audio_src_pad = gst::GhostPad::builder(gst::PadDirection::Src)
                .name(&stream.name)
                .with_target(&audio_tee.request_pad_simple("src_%u").unwrap())
                .unwrap()
                .build();
            media_src.add_pad(&audio_src_pad).unwrap();
            stream.setup(state.clone(), &pipeline, audio_src_pad.upcast_ref(), &path)?;
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
                pipeline.set_state(gst::State::Null)?;
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
                break;
            }
            _ => (),
        }
    }

    pipeline.set_state(gst::State::Null)?;

    Ok(())
}
