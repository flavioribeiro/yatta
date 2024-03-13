use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Error;
use gst::prelude::*;

use crate::{hlscmaf, utils, State};

pub(crate) struct AudioStream {
    pub name: String,
    pub lang: String,
    pub default: bool,
}

impl AudioStream {
    pub fn setup(
        &self,
        state: Arc<Mutex<State>>,
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        path: &Path,
    ) -> Result<(), Error> {
        let queue = gst::ElementFactory::make("queue")
            .name(format!("{}-queue", self.name))
            .build()?;

        let enc = gst::ElementFactory::make("avenc_aac").build()?;
        let mux = gst::ElementFactory::make("cmafmux")
            .property_from_str("header-update-mode", "update")
            .property("write-mehd", true)
            .property("fragment-duration", 2000.mseconds())
            .build()?;
        let appsink = gst_app::AppSink::builder().buffer_list(true).build();

        pipeline.add_many([&queue, &enc, &mux, appsink.upcast_ref()])?;

        src_pad
            .link(&queue.static_pad("sink").unwrap())
            .expect("Failed to link audio queue");

        gst::Element::link_many([&queue, &enc, &mux, appsink.upcast_ref()])?;

        utils::probe_encoder(state, enc, self.name.clone());

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }
}
