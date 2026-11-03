//! Sound reactivity: listens to what the computer plays (the default output's
//! monitor) and keeps 32 log-spaced loudness bands for scenes. Nothing is
//! recorded or stored; only band levels reach the shaders. Runs only while a
//! sound-reactive scene is on screen.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gst::prelude::*;

pub const BANDS: usize = 32;
const RAW_BANDS: usize = 256;
const THRESHOLD_DB: f32 = -70.0;

#[derive(Default)]
pub struct Audio {
    pub bands: RefCell<[f32; BANDS]>,
    pub level: Cell<f32>,
    pub bass: Cell<f32>,
    running: RefCell<Option<(gst::Pipeline, gst::bus::BusWatchGuard)>>,
}

impl Audio {
    pub fn new() -> Rc<Self> {
        Rc::default()
    }

    pub fn set_active(self: &Rc<Self>, active: bool) {
        let running = self.running.borrow().is_some();
        if active && !running {
            if let Err(e) = self.start() {
                eprintln!("frontrise: sound reactivity unavailable: {e}");
            }
        } else if !active && running {
            self.stop();
        }
    }

    fn start(self: &Rc<Self>) -> Result<(), String> {
        let desc = format!(
            "pulsesrc device=@DEFAULT_MONITOR@ client-name=Frontrise ! audioconvert ! \
             audio/x-raw,channels=1 ! spectrum bands={RAW_BANDS} threshold={THRESHOLD_DB} \
             interval=33333333 post-messages=true message-phase=false ! fakesink sync=false"
        );
        let pipeline = gst::parse::launch(&desc)
            .map_err(|e| e.to_string())?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline".to_owned())?;
        let weak: Weak<Audio> = Rc::downgrade(self);
        let bus = pipeline.bus().ok_or("pipeline has no bus")?;
        let guard = bus
            .add_watch_local(move |_, msg| {
                if let Some(audio) = weak.upgrade() {
                    audio.handle(msg);
                }
                glib::ControlFlow::Continue
            })
            .map_err(|e| e.to_string())?;
        pipeline.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
        *self.running.borrow_mut() = Some((pipeline, guard));
        Ok(())
    }

    fn stop(&self) {
        if let Some((pipeline, _guard)) = self.running.borrow_mut().take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
        *self.bands.borrow_mut() = [0.0; BANDS];
        self.level.set(0.0);
        self.bass.set(0.0);
    }

    fn handle(&self, msg: &gst::Message) {
        match msg.view() {
            gst::MessageView::Element(e) => {
                let Some(s) = e.structure() else { return };
                if s.name() != "spectrum" {
                    return;
                }
                let Ok(list) = s.get::<gst::List>("magnitude") else { return };
                let raw: Vec<f32> = list.iter().filter_map(|v| v.get::<f32>().ok()).collect();
                if raw.is_empty() {
                    return;
                }
                self.update(&raw);
            }
            gst::MessageView::Error(e) => {
                eprintln!("frontrise: audio capture error: {}", e.error());
            }
            _ => {}
        }
    }

    /// Groups linear FFT bins into log-spaced bands, normalises dB to 0..1 and
    /// smooths with fast attack / slow release.
    fn update(&self, raw: &[f32]) {
        let n = raw.len();
        let mut bands = self.bands.borrow_mut();
        let mut start = 1usize;
        for (i, band) in bands.iter_mut().enumerate() {
            let end = ((n as f32).powf((i + 1) as f32 / BANDS as f32) as usize).clamp(start + 1, n);
            let peak = raw[start.min(n - 1)..end].iter().copied().fold(THRESHOLD_DB, f32::max);
            let value = ((peak - THRESHOLD_DB) / -THRESHOLD_DB).clamp(0.0, 1.0);
            *band = if value > *band { value } else { *band * 0.85 + value * 0.15 };
            start = end;
        }
        let level = bands.iter().sum::<f32>() / BANDS as f32;
        let bass = bands[..4].iter().sum::<f32>() / 4.0;
        self.level.set(level);
        self.bass.set(bass);
    }
}
