use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use m3u8_rs::{MediaPlaylist, MediaSegment};

use chrono::{Duration, Utc, DateTime};
use gst::prelude::*;
use log::info;

struct StreamState {
    path: PathBuf,
    segments: VecDeque<Segment>,
    trimmed_segments: VecDeque<UnreffedSegment>,
    start_date_time: Option<DateTime<Utc>>,
    start_time: Option<gst::ClockTime>,
    media_sequence: u64,
    segment_index: u32,
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

pub(crate) fn setup(appsink: &gst_app::AppSink, name: &str, path: &Path) {
    let mut path: PathBuf = path.into();
    path.push(name);

    let state = Arc::new(Mutex::new(StreamState {
        segments: VecDeque::new(),
        trimmed_segments: VecDeque::new(),
        path,
        start_date_time: None,
        start_time: gst::ClockTime::NONE,
        media_sequence: 0,
        segment_index: 0,
    }));

    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let mut state = state.lock().unwrap();

                // The muxer only outputs non-empty buffer lists
                let mut buffer_list = sample.buffer_list_owned().expect("no buffer list");
                assert!(!buffer_list.is_empty());

                let mut first = buffer_list.get(0).unwrap();

                // Each list contains a full segment, i.e. does not start with a DELTA_UNIT
                assert!(!first.flags().contains(gst::BufferFlags::DELTA_UNIT));

                // If the buffer has the DISCONT and HEADER flag set then it contains the media
                // header, i.e. the `ftyp`, `moov` and other media boxes.
                //
                // This might be the initial header or the updated header at the end of the stream.
                if first
                    .flags()
                    .contains(gst::BufferFlags::DISCONT | gst::BufferFlags::HEADER)
                {
                    let mut path = state.path.clone();
                    std::fs::create_dir_all(&path).expect("failed to create directory");
                    path.push("init.mp4");

                    info!("writing header to {}", path.display());
                    let map = first.map_readable().unwrap();
                    std::fs::write(path, &map).expect("failed to write header");
                    drop(map);

                    // Remove the header from the buffer list
                    buffer_list.make_mut().remove(0, 1);

                    // If the list is now empty then it only contained the media header and nothing
                    // else.
                    if buffer_list.is_empty() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    // Otherwise get the next buffer and continue working with that.
                    first = buffer_list.get(0).unwrap();
                }

                // If the buffer only has the HEADER flag set then this is a segment header that is
                // followed by one or more actual media buffers.
                assert!(first.flags().contains(gst::BufferFlags::HEADER));

                let mut path = state.path.clone();
                let basename = format!("segment_{}.fmp4", state.segment_index);
                state.segment_index += 1;
                path.push(&basename);

                let segment = sample
                    .segment()
                    .expect("no segment")
                    .downcast_ref::<gst::ClockTime>()
                    .expect("no time segment");
                let pts = segment
                    .to_running_time(first.pts().unwrap())
                    .expect("can't get running time");

                if state.start_time.is_none() {
                    state.start_time = Some(pts);
                }

                if state.start_date_time.is_none() {
                    let now_utc = Utc::now();
                    let now_gst = sink.clock().unwrap().time().unwrap();
                    let pts_clock_time = pts + sink.base_time().unwrap();

                    let diff = now_gst.checked_sub(pts_clock_time).unwrap();
                    let pts_utc = now_utc
                        .checked_sub_signed(Duration::nanoseconds(diff.nseconds() as i64))
                        .unwrap();

                    state.start_date_time = Some(pts_utc);
                }

                let duration = first.duration().unwrap();

                let mut file = std::fs::File::create(&path).expect("failed to open fragment");
                for buffer in &*buffer_list {
                    use std::io::prelude::*;

                    let map = buffer.map_readable().unwrap();
                    file.write_all(&map).expect("failed to write fragment");
                }

                let date_time = state
                    .start_date_time
                    .unwrap()
                    .checked_add_signed(Duration::nanoseconds(
                        pts.opt_checked_sub(state.start_time)
                            .unwrap()
                            .unwrap()
                            .nseconds() as i64,
                    ))
                    .unwrap();

                info!("wrote segment: {}", path.display());

                state.segments.push_back(Segment {
                    duration,
                    path: basename.to_string(),
                    date_time,
                });

                update_manifest(&mut state);

                Ok(gst::FlowSuccess::Ok)
            })
            .eos(move |_sink| {
                unreachable!();
            })
            .build(),
    );
}

fn update_manifest(state: &mut StreamState) {
    // Now write the manifest
    let mut path = state.path.clone();
    path.push("manifest.m3u8");

    trim_segments(state);

    let playlist = MediaPlaylist {
        version: Some(7),
        target_duration: 2.0,
        media_sequence: state.media_sequence,
        segments: state
            .segments
            .iter()
            .enumerate()
            .map(|(idx, segment)| MediaSegment {
                uri: segment.path.to_string(),
                duration: (segment.duration.nseconds() as f64
                    / gst::ClockTime::SECOND.nseconds() as f64) as f32,
                map: if idx == 0 {
                    Some(m3u8_rs::Map {
                        uri: "init.mp4".into(),
                        ..Default::default()
                    })
                } else {
                    None
                },
                program_date_time: if idx == 0 {
                    Some(segment.date_time.into())
                } else {
                    None
                },
                ..Default::default()
            })
            .collect(),
        end_list: false,
        playlist_type: None,
        i_frames_only: false,
        start: None,
        independent_segments: true,
        ..Default::default()
    };

    info!("writing manifest to {}", path.display());
    let mut file = std::fs::File::create(path).unwrap();
    playlist
        .write_to(&mut file)
        .expect("Failed to write media playlist");
}

fn trim_segments(state: &mut StreamState) {
    // Arbitrary 5 segments window
    while state.segments.len() > 5 {
        let segment = state.segments.pop_front().unwrap();

        state.media_sequence += 1;

        state.trimmed_segments.push_back(UnreffedSegment {
            // HLS spec mandates that segments are removed from the filesystem no sooner
            // than the duration of the longest playlist + duration of the segment.
            // This is 15 seconds (12.5 + 2.5) in our case, we use 20 seconds to be on the
            // safe side
            removal_time: segment
                .date_time
                .checked_add_signed(Duration::seconds(20))
                .unwrap(),
            path: segment.path.clone(),
        });
    }

    while let Some(segment) = state.trimmed_segments.front() {
        if segment.removal_time < state.segments.front().unwrap().date_time {
            let segment = state.trimmed_segments.pop_front().unwrap();

            let mut path = state.path.clone();
            path.push(segment.path);
            info!("deleting {}", path.display());
            std::fs::remove_file(path).expect("Failed to remove old segment");
        } else {
            break;
        }
    }
}
