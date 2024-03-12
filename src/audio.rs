use gst::prelude::*;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Error;

use crate::{hlscmaf, utils, State};

pub(crate) struct AudioStream {
    pub name: String,
    pub lang: String,
    pub default: bool,
    pub wave: String,
}

impl AudioStream {
    pub fn setup(
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

        utils::probe_encoder(state, enc, self.name.clone());

        hlscmaf::setup(&appsink, &self.name, path);

        Ok(())
    }
}
