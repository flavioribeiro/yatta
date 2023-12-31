use gst::prelude::*;
use log::info;

use std::collections::HashMap;
use std::path::{PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Error;
use m3u8_rs::{AlternativeMedia, AlternativeMediaType, MasterPlaylist, VariantStream};

mod hlscmaf;
mod utils;
mod video;
mod audio;

struct State {
    video_streams: Vec<video::VideoStream>,
    audio_streams: Vec<audio::AudioStream>,
    all_mimes: HashMap<String, String>,
    path: PathBuf,
    wrote_manifest: bool,
}

impl State {
    fn try_write_manifest(&mut self) {
        if self.wrote_manifest || self.all_mimes.len() < self.video_streams.len() + self.audio_streams.len() { return };
        self.write_manifest()
    }

    fn write_manifest(&mut self) {
        let playlist = MasterPlaylist {
            version: Some(7),
            variants: self.video_streams.iter().map(|stream| {
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
            alternatives: self.audio_streams.iter().map(|stream| {
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
        playlist.write_to(&mut file).expect("Failed to write master playlist");
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
        audio_streams: vec![
            audio::AudioStream {
                name: "audio_0".to_string(),
                lang: "en".to_string(),
                default: true,
                wave: "sine".to_string(),
            },
        ],
        all_mimes: HashMap::new(),
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

    let bus = pipeline.bus().expect("Pipeline without bus. Shouldn't happen!");

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
                    msg.src().map(|s| String::from(s.path_string())).unwrap_or_else(|| "None".into()),
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
