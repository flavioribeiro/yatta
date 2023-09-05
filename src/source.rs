use gst::prelude::*;

use anyhow::Error;
use gst::Element;


pub(crate) fn create_source_tees(
    _srt_url: String, 
    pipeline: &gst::Pipeline
) -> Result<(Element, Element), Error> {
    let video_src = gst::ElementFactory::make("videotestsrc")
        .property("is-live", true).build()?;
    let video_tee: Element = gst::ElementFactory::make("tee")
        .property("name", "video_tee")
        .build()?;

    let audio_src = gst::ElementFactory::make("audiotestsrc")
        .property("is-live", true)
        .property_from_str("wave", &"sine")
        .build()?;
    let audio_tee: Element = gst::ElementFactory::make("tee")
        .property("name", "audio_tee")
        .build()?;

    pipeline.add_many(&[&video_src, &video_tee, &audio_src, &audio_tee])?;
    gst::Element::link_many(&[&video_src, &video_tee])?;
    gst::Element::link_many(&[&audio_src, &audio_tee])?;
    

    Ok((video_tee, audio_tee))
}

