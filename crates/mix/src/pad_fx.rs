use super::*;

struct PadChain {
    slots: Vec<Option<PluginSlot>>,
    align: DelayLine,
    latency: usize,
}

pub(super) struct PadEffects {
    pads: [PadChain; 16],
    scratch: Vec<Vec<f32>>,
    pub latency: usize,
    pub enabled: bool,
}

impl Default for PadEffects {
    fn default() -> Self {
        Self {
            pads: std::array::from_fn(|_| PadChain { slots: Vec::new(), align: DelayLine::default(), latency: 0 }),
            scratch: Vec::new(), latency: 0, enabled: false,
        }
    }
}

impl PadEffects {
    pub fn reserve(&mut self, budget: usize) {
        if self.enabled {
            for pad in &mut self.pads { pad.align.reserve(budget, self.scratch.len()); }
        }
    }
    pub fn capacity_exceeded(&self) -> bool { self.pads.iter().any(|pad| pad.align.overflow) }
    pub fn inherit(&mut self, previous: &mut Self, track: TrackId, fresh: &[(TrackId, usize)]) {
        for (index, (next, old)) in self.pads.iter_mut().zip(&mut previous.pads).enumerate() {
            for (slot, (next, old)) in next.slots.iter_mut().zip(&mut old.slots).enumerate() {
                if !fresh.contains(&(track, drum_pad_slot((index + 36) as u8, slot).unwrap_or(INSTRUMENT_SLOT))) { inherit_slot(next, old); }
            }
            inherit_delay(&mut next.align, &mut old.align);
        }
    }
    pub fn retire(&mut self, context: &mut SlotCtx<'_>) {
        for pad in &mut self.pads {
            for slot in pad.slots.drain(..) { context.retire(slot); }
        }
        self.enabled = false;
    }

    pub fn drain(&mut self, output: &mut Vec<Box<dyn Plugin>>) {
        for pad in &mut self.pads {
            output.extend(pad.slots.iter_mut().filter_map(Option::take).map(|slot| slot.plugin));
        }
    }

    pub fn reset(&mut self) {
        for pad in &mut self.pads {
            pad.align.clear();
            for slot in pad.slots.iter_mut().flatten() {
                slot.plugin.reset();
                slot.failed = slot.pending;
            }
        }
    }

    pub fn sync(&mut self, context: &mut SlotCtx<'_>, track: &Track, channels: usize) {
        let instrument = track.instrument.as_ref().filter(|insert| insert.plugin == "drum_rack");
        self.enabled = instrument.is_some_and(|insert| insert.pad_effects.values().any(|slots| slots.iter().any(Option::is_some)));
        if self.enabled && (self.scratch.len() != channels || self.scratch.first().map_or(0, Vec::len) != context.mb) {
            self.scratch = vec![vec![0.0; context.mb]; channels];
        }
        for (index, pad) in self.pads.iter_mut().enumerate() {
            let note = (index + 36) as u8;
            let wanted = instrument.and_then(|insert| insert.pad_effects.get(&note));
            if wanted.is_none() {
                for slot in pad.slots.drain(..) { context.retire(slot); }
                continue;
            }
            pad.slots.resize_with(soundcraft_model::INSERT_SLOTS, || None);
            for (index, slot) in pad.slots.iter_mut().enumerate() {
                let address = drum_pad_slot(note, index).unwrap_or(INSTRUMENT_SLOT);
                let insert = wanted.and_then(|slots| slots.get(index)).and_then(Option::as_ref);
                match insert {
                    Some(insert) if slot.as_ref().is_none_or(|slot| slot.id != insert.plugin || slot.ch != channels)
                        || context.waiting(track.id, address) => {
                        context.retire(slot.take());
                        *slot = context.make(track.id, address, insert, channels);
                    }
                    None => context.retire(slot.take()),
                    _ => {}
                }
            }
        }
        self.prepare(track);
    }

    pub fn prepare(&mut self, track: &Track) {
        self.latency = 0;
        for (index, pad) in self.pads.iter_mut().enumerate() {
            pad.latency = 0;
            let wanted = track.instrument.as_ref().and_then(|insert| insert.pad_effects.get(&((index + 36) as u8)));
            for (slot, insert) in pad.slots.iter_mut().zip(wanted.into_iter().flatten()) {
                if let (Some(slot), Some(insert)) = (slot, insert)
                    && insert.active && !insert.bypass && !slot.failed
                {
                    apply_values(slot, insert, |_, value| value);
                    pad.latency = pad.latency.saturating_add(slot.plugin.latency()).min(MAX_LATENCY);
                }
            }
            self.latency = self.latency.max(pad.latency);
        }
        for pad in &mut self.pads {
            pad.align.set(self.latency.saturating_sub(pad.latency), self.scratch.len());
        }
    }

    pub fn tail(&self, instrument: &soundcraft_model::Insert, note: u8) -> usize {
        let Some(index) = note.checked_sub(36).map(usize::from) else { return 0 };
        let Some(pad) = self.pads.get(index) else { return 0 };
        let Some(wanted) = instrument.pad_effects.get(&note) else { return self.latency };
        pad.slots.iter().zip(wanted).filter_map(|(slot, insert)| {
            let (Some(slot), Some(insert)) = (slot, insert) else { return None };
            (insert.active && !insert.bypass && !slot.failed).then(|| slot.plugin.tail_samples())
        }).fold(self.latency, usize::saturating_add)
    }

    pub fn process(&mut self, instrument: &mut PluginSlot, model: &soundcraft_model::Insert,
        transport: &Transport, output: &mut [Vec<f32>], frames: usize, failures: &mut PluginFailures)
    {
        for channel in output.iter_mut() { channel.iter_mut().take(frames).for_each(|sample| *sample = 0.0); }
        for (index, pad) in self.pads.iter_mut().enumerate() {
            let note = (index + 36) as u8;
            for channel in &mut self.scratch { channel.iter_mut().take(frames).for_each(|sample| *sample = 0.0); }
            instrument.plugin.process_pad(note, &mut self.scratch, frames);
            if contain_failure(instrument, &mut self.scratch, frames) {
                failures.record(INSTRUMENT_SLOT);
            }
            let wanted = model.pad_effects.get(&note);
            for (index, (slot, insert)) in pad.slots.iter_mut().zip(wanted.into_iter().flatten()).enumerate() {
                if let (Some(slot), Some(insert)) = (slot, insert)
                    && insert.active && !insert.bypass && !slot.failed
                {
                    slot.plugin.set_transport(transport);
                    slot.plugin.process(&mut self.scratch, frames);
                    if contain_failure(slot, &mut self.scratch, frames) {
                        failures.record(drum_pad_slot(note, index).unwrap_or(INSTRUMENT_SLOT));
                    }
                }
            }
            pad.align.process(&mut self.scratch, frames);
            let mute = instrument.plugin.param(format_key(note, "mute").as_str()).unwrap_or(0.0) >= 0.5;
            let pan = instrument.plugin.param(format_key(note, "pan").as_str()).unwrap_or(0.0);
            if mute { continue; }
            for (channel, (output, input)) in output.iter_mut().zip(&self.scratch).enumerate() {
                let gain = match channel { 0 => (1.0 - pan).min(1.0), 1 => (1.0 + pan).min(1.0), _ => 1.0 };
                for (sample, input) in output.iter_mut().zip(input).take(frames) { *sample += input * gain; }
            }
        }
    }
}

fn format_key(note: u8, parameter: &str) -> Key {
    Key::new(format_args!("pad{note}.{parameter}")).unwrap_or(Key { buf: [0; 64], len: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HostedEffect { delay: DelayLine, invalid: bool }
    impl Plugin for HostedEffect {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo { soundcraft_dsp::plugin_info("gain").unwrap() }
        fn prepare(&mut self, _rate:f32, _block:usize, channels:usize) { self.delay.set(7,channels); }
        fn reset(&mut self) { self.delay.clear(); }
        fn set_param(&mut self, _name:&str, _value:f32) -> bool { true }
        fn param(&self, _name:&str) -> Option<f32> { None }
        fn process(&mut self, audio:&mut [Vec<f32>], frames:usize) {
            if self.invalid { for channel in audio { channel.iter_mut().take(frames).for_each(|sample|*sample=f32::NAN); } }
            else { self.delay.process(audio,frames); }
        }
        fn latency(&self) -> usize { 7 }
        fn tail_samples(&self) -> usize { 19 }
    }

    fn fixture(invalid:bool) -> (Session,TrackId,MixEngine) {
        let mut session = Session::default();
        let source = soundcraft_model::SourceId(1100);
        session.pool.insert(source,Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer {
            sample_rate:48_000,channels:vec![vec![0.25;4800]],
        })));
        let track = session.add_track(TrackKind::Instrument,ChannelFormat::Stereo,None);
        let mut rack = soundcraft_model::Insert::new("drum_rack");
        rack.pad_samples.insert(36,source);
        rack.pad_samples.insert(37,source);
        let mut slots = std::array::from_fn(|_|None);
        slots[2] = Some(soundcraft_model::Insert::new("clap:test.pad"));
        rack.pad_effects.insert(36,slots);
        session.track_mut(track).unwrap().instrument=Some(rack);
        let specs=instance_specs(&session);
        assert_eq!(specs.len(),1);
        assert_eq!(specs[0].slot,drum_pad_slot(36,2).unwrap());
        let mut plugin=HostedEffect { delay:DelayLine::default(),invalid };
        plugin.prepare(session.sample_rate.as_f64() as f32,128,2);
        let mut mixer=MixEngine::new(session.sample_rate.as_f64() as f32,128);
        mixer.set_external_instances(true);
        mixer.adopt(vec![PreparedInstance { track,slot:specs[0].slot,id:specs[0].id.clone(),ch:2,plugin:Box::new(plugin) }]);
        mixer.monitor_only=true;
        (session,track,mixer)
    }

    #[test]
    fn hosted_pad_slots_adopt_align_tail_and_retire_through_the_existing_host() {
        let (session,track,mut mixer)=fixture(false);
        let frames=mixer.audition(&session,track,36);
        assert_eq!(frames,4800+7+19);
        let mut audio=vec![vec![0.0;128];2];
        mixer.render(&session,0,128,&mut audio);
        assert_eq!(mixer.latency,7);
        let strip=mixer.strips.get(&track).unwrap();
        assert_eq!(strip.pad_effects.pads[1].align.len,7);
        let mut edited=session.clone();
        edited.track_mut(track).unwrap().instrument.as_mut().unwrap().pad_effects.clear();
        mixer.ensure_synced(&edited);
        assert_eq!(mixer.take_retired().len(),1);
    }

    #[test]
    fn invalid_pad_processing_reports_the_exact_slot_and_other_pads_keep_sounding() {
        let (session,track,mut mixer)=fixture(true);
        mixer.audition(&session,track,36);
        mixer.audition(&session,track,37);
        let mut audio=vec![vec![0.0;128];2];
        mixer.render(&session,0,128,&mut audio);
        assert!(audio.iter().flatten().all(|sample|sample.is_finite()));
        assert!(audio.iter().flatten().any(|sample|sample.abs()>0.01));
        let strip=mixer.strips.get(&track).unwrap();
        assert_eq!(strip.failures.last,drum_pad_slot(36,2).unwrap());
        assert_eq!(strip.failures.count,1);
    }

    #[test]
    fn pad_addresses_are_disjoint_from_track_inserts_and_round_trip() {
        for note in 36..=51 {
            for slot in 0..soundcraft_model::INSERT_SLOTS {
                let address=drum_pad_slot(note,slot).unwrap();
                assert_eq!(decode_drum_pad_slot(address),Some((note,slot)));
                assert!(address>=soundcraft_model::INSERT_SLOTS && address!=INSTRUMENT_SLOT);
            }
        }
        assert_eq!(decode_drum_pad_slot(INSTRUMENT_SLOT),None);
        assert_eq!(drum_pad_slot(35,0),None);
        assert_eq!(drum_pad_slot(36,10),None);
        assert_eq!(drum_pad_slot(36,usize::MAX),None);
    }
}

