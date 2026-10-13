use crate::{audio::midi::{MidiChannel, SongMidiOps}, model::{song::Song, track::{Track, SongTrackOps}, rse::SongRseOps}};

fn song_with_channels() -> Song {
    let mut song = Song::default();
    song.channels = (0..64).map(|channel| MidiChannel { channel, effect_channel: channel, ..Default::default() }).collect();
    song
}

#[test]
fn track_routing_follows_encoded_channel_not_track_number() {
    for version in [(3,0,0), (4,0,0), (5,0,0), (5,1,0)] {
        for channel in [3, 9, 25] {
            let mut source = song_with_channels();
            source.version.number = version;
            source.channels[channel].instrument = 33;
            source.channels[channel].bank = 7;
            source.tracks.push(Track { channel_index: channel, name: "Routing fixture".into(), strings: vec![(1,64)], ..Default::default() });
            let mut bytes = Vec::new();
            if version.0 < 5 { source.write_track(&mut bytes, 0); }
            else { source.write_track_v5(&mut bytes, 0, &version); }
            let mut parsed = song_with_channels();
            parsed.version.number = version;
            parsed.channels[channel].instrument = 33;
            let mut cursor = 0;
            if version.0 < 5 { parsed.read_track(&bytes, &mut cursor, 0).unwrap(); }
            else { parsed.read_track_v5(&bytes, &mut cursor, 0).unwrap(); }
            assert_eq!(cursor, bytes.len());
            assert_eq!(parsed.tracks[0].channel_index, channel);
            assert_eq!(parsed.channels[parsed.tracks[0].channel_index].instrument, 33);
            assert_eq!(parsed.tracks[0].percussion_track, channel % 16 == 9);
            if version.0 == 5 {
                assert_eq!(parsed.channels[channel].bank, 7);
                assert_eq!(parsed.channels[0].bank, 0);
            }
        }
    }
}

#[test]
fn invalid_track_channels_return_errors() {
    for (channel, effect) in [(0,1), (-1,1), (65,1), (1,0), (1,65)] {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&i32::to_le_bytes(channel));
        bytes.extend_from_slice(&i32::to_le_bytes(effect));
        assert!(song_with_channels().read_channel(&bytes, &mut 0).is_err());
    }
}

#[test]
fn equalizer_separates_pre_gain_and_roundtrips_bytes() {
    let song = Song::default();
    for count in [4,11] {
        let bytes: Vec<u8> = (0..count).map(|i| (i as i8 * 3 - 12) as u8).collect();
        let mut cursor = 0;
        let eq = song.read_rse_equalizer(&bytes, &mut cursor, count).unwrap();
        assert_eq!(eq.knobs.len(), count as usize - 1);
        assert_eq!(eq.gain, -(bytes[count as usize - 1] as i8 as f32) / 10.0);
        assert_eq!(cursor, bytes.len());
        let mut written = Vec::new();
        song.write_equalizer(&mut written, &eq);
        assert_eq!(written, bytes);
    }
}

#[test]
fn unknown_rse_words_are_preserved_without_interpretation() {
    let mut song = song_with_channels();
    song.version.number = (5,1,0);
    song.master_effect.volume = 100.0;
    song.master_effect.raw_master_value = 123456;
    let mut bytes = Vec::new();
    song.write_rse_master_effect(&mut bytes);
    let mut cursor = 0;
    let master = song.read_rse_master_effect(&bytes, &mut cursor).unwrap();
    assert_eq!(master.raw_master_value, 123456);
    assert_eq!(cursor, bytes.len());
    let mut track = Track::default();
    track.rse.raw_values = Some([1, -2, 100, 4, 5, 6]);
    bytes.clear();
    song.write_track_rse(&mut bytes, &track.rse, &song.version.number);
    let mut parsed = Track::default();
    cursor = 0;
    song.read_track_rse(&bytes, &mut cursor, &mut parsed).unwrap();
    assert_eq!(parsed.rse.raw_values, track.rse.raw_values);
    assert_eq!(cursor, bytes.len());
    let mut written = Vec::new();
    song.write_track_rse(&mut written, &parsed.rse, &song.version.number);
    assert_eq!(written, bytes);
    assert!(song.read_track_rse(&bytes[..12], &mut 0, &mut Track::default()).is_err());
}
