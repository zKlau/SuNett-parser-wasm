// GPIF Song conversion - Main implementation
use super::beat::convert_beat;
use super::helpers::*;
use crate::io::gpif::model::*;
use crate::model::{
    beat::Voice as SongVoice,
    headers::{Marker, MeasureFermata, MeasureHeader},
    measure::Measure,
    song::*,
    track::Track as SongTrack,
};
use crate::types::effects::*;
use std::collections::HashMap;

pub trait SongGpifOps {
    fn read_gpif(&mut self, gpif: &Gpif);
}

impl SongGpifOps for Song {
    fn read_gpif(&mut self, gpif: &Gpif) {
        // 0. Version
        let default_version = self.version.number;
        self.version.number = parse_gpif_version(gpif, default_version);

        // 1. Metadata
        self.name = gpif.score.title.clone();
        self.subtitle = gpif.score.sub_title.clone();
        self.artist = gpif.score.artist.clone();
        self.album = gpif.score.album.clone();
        self.words = gpif.score.words.clone();
        self.author = gpif.score.music.clone();
        self.writer = gpif.score.music.clone();
        self.transcriber = gpif.score.tabber.clone();
        self.copyright = gpif.score.copyright.clone();
        self.comments = gpif.score.instructions.clone();
        // Notices
        if !gpif.score.notices.is_empty() {
            self.notice = gpif.score.notices.lines().map(|l| l.to_string()).collect();
        }

        // 2. Tempo from MasterTrack automations
        if let Some(automations) = &gpif.master_track.automations {
            for auto in &automations.automations {
                if auto.automation_type == "Tempo" && auto.bar == 0 {
                    if let Some(tempo_str) = auto.value.split_whitespace().next() {
                        self.tempo = match tempo_str.parse::<f64>() {
                            Ok(v) => v as i16,
                            Err(_) => 120,
                        };
                    }
                }
            }
        }

        // 2b. Master RSE
        if let Some(rse_wrapper) = &gpif.master_track.rse {
            if let Some(master) = &rse_wrapper.master {
                if let Some(vol) = master.volume {
                    // GPX volume is typically 0.0-1.0 or similar.
                    // Scorelib model expects arbitrary flow. we store as is.
                    self.master_effect.volume = vol * 100.0; // rudimentary mapping
                }
            }
        }

        // 3. Build lookup maps
        let bars_map: HashMap<i32, &Bar> = gpif.bars.bars.iter().map(|b| (b.id, b)).collect();
        let voices_map: HashMap<i32, &Voice> =
            gpif.voices.voices.iter().map(|v| (v.id, v)).collect();
        let beats_map: HashMap<i32, &Beat> = gpif.beats.beats.iter().map(|b| (b.id, b)).collect();
        let notes_map: HashMap<i32, &Note> = gpif.notes.notes.iter().map(|n| (n.id, n)).collect();
        let rhythms_map: HashMap<i32, &Rhythm> =
            gpif.rhythms.rhythms.iter().map(|r| (r.id, r)).collect();

        // 4. Measure Headers (MasterBars) — also collects per-track bar IDs
        self.measure_headers.clear();
        let num_tracks = gpif.tracks.tracks.len();
        let mut track_bar_ids: Vec<Vec<i32>> = vec![Vec::new(); num_tracks];

        for (mh_idx, mb) in gpif.master_bars.master_bars.iter().enumerate() {
            let mut mh = MeasureHeader {
                number: (mh_idx + 1) as u16,
                ..Default::default()
            };

            // Time signature
            let time_parts: Vec<&str> = mb.time.split('/').collect();
            if time_parts.len() == 2 {
                mh.time_signature.numerator = time_parts[0].parse().unwrap_or(4) as i8;
                mh.time_signature.denominator.value = time_parts[1].parse().unwrap_or(4) as u16;
            }

            // Key signature
            if let Some(key) = &mb.key {
                mh.key_signature.key = key.accidental_count as i8;
                mh.key_signature.is_minor = key.mode == "Minor";
            }

            // Tempo at this bar
            if let Some(automations) = &gpif.master_track.automations {
                for auto in &automations.automations {
                    if auto.automation_type == "Tempo" && auto.bar == mh_idx as i32 {
                        if let Some(tempo_str) = auto.value.split_whitespace().next() {
                            mh.tempo = tempo_str.parse::<f64>().unwrap_or(0.0) as i32;
                        }
                    }
                }
            }

            // Repeat
            if let Some(repeat) = &mb.repeat {
                mh.repeat_open = repeat.start == "true";
                if repeat.end == "true" {
                    mh.repeat_close = repeat.count.max(1) as i8;
                }
            }

            // Alternate endings (volta)
            if let Some(alt_str) = &mb.alternate_endings {
                let mut bitmask: u8 = 0;
                for tok in alt_str.split_whitespace() {
                    if let Ok(n) = tok.parse::<u8>() {
                        if n > 0 && n <= 8 {
                            bitmask |= 1 << (n - 1);
                        }
                    }
                }
                mh.repeat_alternative = bitmask;
            }

            // Double bar
            mh.double_bar = mb.double_bar.is_some();

            // Marker (Section)
            if let Some(section) = &mb.section {
                let title = section
                    .text
                    .as_deref()
                    .unwrap_or(section.letter.as_deref().unwrap_or("Section"));
                // GP6/7 GPIF XML does not include marker color; use the default (red).
                mh.marker = Some(Marker {
                    title: title.to_string(),
                    color: 0xff0000,
                });
            }

            // Fermatas
            if let Some(fermatas_w) = &mb.fermatas {
                for f in &fermatas_w.fermatas {
                    let ftype = parse_fermata_type(f.fermata_type.as_deref().unwrap_or("Medium"));
                    let offset = parse_fraction_offset(f.offset.as_deref().unwrap_or("0/1"));
                    mh.fermatas.push(MeasureFermata {
                        fermata_type: ftype,
                        offset,
                    });
                }
            }

            // Free time
            mh.free_time = mb.free_time.is_some();

            // Directions
            if let Some(dirs) = &mb.directions {
                if let Some(target) = &dirs.target {
                    mh.direction = parse_direction_sign(target);
                } else if let Some(jump) = &dirs.jump {
                    mh.direction = parse_direction_sign(jump);
                }
            }

            // Per-track bar IDs
            let bar_ids = parse_ids(&mb.bars);
            for (t_idx, &bar_id) in bar_ids.iter().enumerate() {
                if t_idx < num_tracks {
                    track_bar_ids[t_idx].push(bar_id);
                }
            }

            self.measure_headers.push(mh);
        }

        let num_measures = self.measure_headers.len();

        // 5. Tracks
        self.tracks.clear();

        for (t_idx, g_track) in gpif.tracks.tracks.iter().enumerate() {
            let mut track = SongTrack {
                name: g_track.name.clone(),
                short_name: g_track.short_name.clone(),
                number: (t_idx + 1) as i32,
                ..Default::default()
            };

            // Color
            if let Some(color_str) = &g_track.color {
                let rgb: Vec<i32> = color_str
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if rgb.len() == 3 {
                    track.color = rgb[0] * 65536 + rgb[1] * 256 + rgb[2];
                }
            }

            // Tuning: GP6 track-level properties, GP7 staves
            track.strings.clear();
            if let Some(props) = &g_track.properties {
                track.strings = extract_tuning(&props.properties);
            }
            if track.strings.is_empty() {
                if let Some(staves) = &g_track.staves {
                    for staff in &staves.staves {
                        if let Some(props) = &staff.properties {
                            track.strings = extract_tuning(&props.properties);
                            if !track.strings.is_empty() {
                                break;
                            }
                        }
                    }
                }
            }
            if track.strings.is_empty() {
                track.strings = vec![(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)];
            }

            track.fret_count = 24;

            // MIDI: GP6 uses <GeneralMidi>, GP7 uses <MidiConnection>
            if let Some(gm) = &g_track.general_midi {
                if let Some(ch) = gm.primary_channel {
                    track.channel_index = ch as usize;
                    track.percussion_track = ch == 9;
                }
                track.midi_program_gpif = gm.program;
                if let Some(port) = gm.port {
                    track.port = port as u8;
                }
            }
            if let Some(mc) = &g_track.midi_connection {
                if let Some(ch) = mc.primary_channel {
                    track.channel_index = ch as usize;
                    track.percussion_track = ch == 9;
                }
                if let Some(port) = mc.port {
                    track.port = port as u8;
                }
            }
            // A drumKit instrument set is percussion regardless of MIDI channel.
            if let Some(iset) = &g_track.instrument_set {
                if iset.instrument_type == "drumKit" {
                    track.percussion_track = true;
                }
            }

            // Transpose
            if let Some(tr) = &g_track.transpose {
                track.transpose_chromatic = tr.chromatic.unwrap_or(0);
                track.transpose_octave = tr.octave.unwrap_or(0);
            }

            // Current dynamic (persists across beats)
            let mut current_velocity: i16 = FORTE;

            // RSE (GPX)
            if let Some(rse_wrapper) = &g_track.rse {
                // Populate humanize/instrument from GPX RSE if possible
                // Currently just setting default or mapping names if available
                if let Some(chains) = &rse_wrapper.effect_chains {
                    if let Some(first_chain) = chains.effect_chains.first() {
                        track.rse.instrument.effect_category = first_chain.name.clone();
                    }
                }
            }

            // Measures
            for m_idx in 0..num_measures {
                let mut measure = Measure {
                    number: m_idx + 1,
                    track_index: t_idx,
                    ..Default::default()
                };

                if m_idx < self.measure_headers.len() {
                    measure.time_signature = self.measure_headers[m_idx].time_signature.clone();
                    measure.key_signature = self.measure_headers[m_idx].key_signature.clone();
                }

                let bar_id = if m_idx < track_bar_ids[t_idx].len() {
                    track_bar_ids[t_idx][m_idx]
                } else {
                    -1
                };

                if let Some(bar) = bars_map.get(&bar_id) {
                    measure.simile_mark = bar.simile_mark.clone();
                    let voice_ids = parse_ids(&bar.voices);
                    measure.voices.clear();

                    for &vid in &voice_ids {
                        if vid < 0 {
                            continue;
                        }
                        let mut s_voice = SongVoice::default();

                        if let Some(g_voice) = voices_map.get(&vid) {
                            let beat_ids = parse_ids(&g_voice.beats);

                            for &bid in &beat_ids {
                                if let Some(g_beat) = beats_map.get(&bid) {
                                    let s_beat = convert_beat(
                                        g_beat,
                                        &rhythms_map,
                                        &notes_map,
                                        &mut current_velocity,
                                    );
                                    s_voice.beats.push(s_beat);
                                }
                            }
                        }
                        measure.voices.push(s_voice);
                    }
                }
                track.measures.push(measure);
            }
            self.tracks.push(track);
        }
    }
}
