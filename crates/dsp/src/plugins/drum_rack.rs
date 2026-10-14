use super::sampler::Sampler;
use crate::{Category, Plugin, PluginInfo, SampleSource};
use std::sync::Arc;

pub static DRUM_RACK_INFO: PluginInfo = PluginInfo {
    id: "drum_rack",
    name: "Drum Rack",
    short_name: "DrumRack",
    category: Category::Instrument,
    params: &[],
    is_instrument: true,
    audiosuite: false,
};

pub struct DrumRack {
    pads: [Sampler; 16],
    pan: [f32; 16],
    mute: [bool; 16],
    scratch: Vec<Vec<f32>>,
}

impl Default for DrumRack {
    fn default() -> Self {
        Self::new()
    }
}

impl DrumRack {
    pub fn new() -> Self {
        Self {
            pads: std::array::from_fn(|_| Sampler::new()),
            pan: [0.0; 16],
            mute: [false; 16],
            scratch: Vec::new(),
        }
    }

    fn pad_index(note: u8) -> Option<usize> {
        note.checked_sub(36).filter(|index| *index < 16).map(usize::from)
    }

    fn control(id: &str) -> Option<(usize, &str)> {
        let (note, param) = id.strip_prefix("pad")?.split_once('.')?;
        Some((Self::pad_index(note.parse().ok()?)?, param))
    }
}

impl Plugin for DrumRack {
    fn info(&self) -> &'static PluginInfo {
        &DRUM_RACK_INFO
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.scratch = vec![vec![0.0; max_block.max(1)]; channels.clamp(1, 16)];
        for (index, pad) in self.pads.iter_mut().enumerate() {
            pad.prepare(sample_rate, max_block, self.scratch.len());
            pad.set_param("root_note", (36 + index) as f32);
            pad.set_param("key_track", 0.0);
            pad.set_param("play_mode", 1.0);
        }
    }

    fn reset(&mut self) {
        for pad in &mut self.pads {
            pad.reset();
        }
    }

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        let Some((index, param)) = Self::control(id) else { return false };
        let Some(pad) = self.pads.get_mut(index) else { return false };
        match param {
            "gain" => pad.set_param("level", value),
            "tune" => pad.set_param("transpose", value),
            "pan" => {
                if let Some(pan) = self.pan.get_mut(index) {
                    *pan = if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 };
                }
                true
            }
            "mute" => {
                if let Some(mute) = self.mute.get_mut(index) {
                    *mute = value.is_finite() && value >= 0.5;
                }
                true
            }
            _ => false,
        }
    }

    fn param(&self, id: &str) -> Option<f32> {
        let (index, param) = Self::control(id)?;
        let pad = self.pads.get(index)?;
        match param {
            "gain" => pad.param("level"),
            "tune" => pad.param("transpose"),
            "pan" => self.pan.get(index).copied(),
            "mute" => self.mute.get(index).map(|muted| if *muted { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let frames = frames.min(self.scratch.first().map_or(0, Vec::len));
        for channel in io.iter_mut() {
            channel.iter_mut().take(frames).for_each(|sample| *sample = 0.0);
        }
        for (index, pad) in self.pads.iter_mut().enumerate() {
            pad.process(&mut self.scratch, frames);
            if self.mute.get(index).copied().unwrap_or(true) {
                continue;
            }
            let pan = self.pan.get(index).copied().unwrap_or(0.0);
            for (channel, (output, input)) in io.iter_mut().zip(&self.scratch).enumerate() {
                let gain = match channel {
                    0 => (1.0 - pan).min(1.0),
                    1 => (1.0 + pan).min(1.0),
                    _ => 1.0,
                };
                for (sample, value) in output.iter_mut().zip(input).take(frames) {
                    *sample += *value * gain;
                }
            }
        }
    }

    fn tail_samples(&self) -> usize {
        self.pads.iter().map(|pad| pad.tail_samples()).max().unwrap_or(0)
    }

    fn note_on(&mut self, offset: usize, note: u8, velocity: u8) {
        if let Some(pad) = Self::pad_index(note).and_then(|index| self.pads.get_mut(index)) {
            pad.note_on(offset, note, velocity);
        }
    }

    fn note_off(&mut self, offset: usize, note: u8) {
        if let Some(pad) = Self::pad_index(note).and_then(|index| self.pads.get_mut(index)) {
            pad.note_off(offset, note);
        }
    }

    fn all_notes_off(&mut self) {
        for pad in &mut self.pads {
            pad.all_notes_off();
        }
    }

    fn set_pad_sample(&mut self, note: u8, sample: Option<Arc<dyn SampleSource>>) -> bool {
        Self::pad_index(note).and_then(|index| self.pads.get_mut(index)).is_some_and(|pad| pad.set_sample(sample))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SampleBuffer;

    #[test]
    fn samples_follow_their_pad_note_and_reset_silences_all_pads() {
        let mut rack = DrumRack::new();
        rack.prepare(48_000.0, 128, 2);
        let sample = Arc::new(SampleBuffer { sample_rate: 48_000.0, channels: vec![vec![0.5; 4800]] });
        assert!(rack.set_pad_sample(36, Some(sample)));
        assert!(!rack.set_pad_sample(35, None));
        let mut out = vec![vec![0.0; 128]; 2];
        rack.note_on(0, 37, 127);
        rack.process(&mut out, 128);
        assert!(out.iter().flatten().all(|value| value.abs() < 1e-6));
        rack.note_on(0, 36, 127);
        rack.process(&mut out, 128);
        assert!(out.iter().flatten().any(|value| value.abs() > 0.1));
        rack.reset();
        rack.process(&mut out, 128);
        assert!(out.iter().flatten().all(|value| value.abs() < 1e-6));
    }

    #[test]
    fn mute_and_pan_change_the_render_without_changing_other_pads() {
        let mut rack = DrumRack::new();
        rack.prepare(48_000.0, 128, 2);
        let sample = Arc::new(SampleBuffer { sample_rate: 48_000.0, channels: vec![vec![0.5; 4800]] });
        rack.set_pad_sample(36, Some(sample));
        assert!(rack.set_param("pad36.pan", 1.0));
        rack.note_on(0, 36, 127);
        let mut out = vec![vec![0.0; 128]; 2];
        rack.process(&mut out, 128);
        assert!(out[0].iter().all(|value| value.abs() < 1e-6));
        assert!(out[1].iter().any(|value| value.abs() > 0.1));
        rack.set_param("pad36.mute", 1.0);
        rack.process(&mut out, 128);
        assert!(out.iter().flatten().all(|value| value.abs() < 1e-6));
        assert!(!rack.set_param("pad200.gain", 0.0));
    }
}
