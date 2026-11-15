use std::time::Instant;

use lumencraft_palette::colormap::{ColorRampError, MAX_CUSTOM_STOPS};
use lumencraft_palette::{ColorRamp, ColorStop, Rgba8};
use serde::Deserialize;
use serde::de::value::Error as DeValueError;
use serde::de::{Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};

fn reference_linear_sample(stops: &[ColorStop], t: f64) -> Rgba8 {
    let mut valid: Vec<ColorStop> = stops
        .iter()
        .copied()
        .filter(|s| s.position.is_finite())
        .map(|s| ColorStop { position: s.position.clamp(0.0, 1.0), color: s.color })
        .collect();
    valid.sort_by(|a, b| a.position.total_cmp(&b.position));
    let Some(&first) = valid.first() else {
        return Rgba8::BLACK;
    };
    if t <= first.position {
        return first.color;
    }
    for pair in valid.windows(2) {
        let [left, right] = pair else { continue };
        if t <= right.position {
            let span = right.position - left.position;
            if span <= 1e-12 {
                return right.color;
            }
            return lumencraft_palette::lerp_oklab(left.color, right.color, (t - left.position) / span);
        }
    }
    valid.last().map_or(first.color, |last| last.color)
}

#[test]
fn r1_s01_07_custom_ramp_sample_matches_reference_for_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
    let stops = vec![
        ColorStop { position: f64::NAN, color: Rgba8::rgb(1, 2, 3) },
        ColorStop { position: 0.5, color: Rgba8::rgb(255, 0, 0) },
        ColorStop { position: 0.5, color: Rgba8::rgb(128, 128, 0) },
        ColorStop { position: 0.5, color: Rgba8::rgb(0, 255, 0) },
        ColorStop { position: -0.25, color: Rgba8::rgb(10, 20, 30) },
        ColorStop { position: 1.5, color: Rgba8::rgb(240, 230, 220) },
        ColorStop { position: f64::INFINITY, color: Rgba8::WHITE },
    ];
    let raw_ramp = ColorRamp::Custom { stops: stops.clone() };
    let mut expected_buf = stops.clone();
    expected_buf.retain(|s| s.position.is_finite());

    for step in 0..=200 {
        let t = (step as f64) / 200.0;
        assert_eq!(raw_ramp.sample(t), reference_linear_sample(&stops, t), "mismatch at t={t}");
    }

    // Also validate hard step discontinuities through normalized constructor.
    let normalized = ColorRamp::custom(vec![
        ColorStop { position: 0.5, color: Rgba8::rgb(255, 0, 0) },
        ColorStop { position: 0.5, color: Rgba8::rgb(10, 20, 30) },
        ColorStop { position: 0.5, color: Rgba8::rgb(0, 0, 255) },
        ColorStop { position: 0.0, color: Rgba8::BLACK },
        ColorStop { position: 1.0, color: Rgba8::WHITE },
    ])?;
    assert_eq!(
        normalized.sample(0.4999999),
        ColorRamp::Custom {
            stops: vec![
                ColorStop { position: 0.0, color: Rgba8::BLACK },
                ColorStop { position: 0.5, color: Rgba8::rgb(255, 0, 0) },
                ColorStop { position: 0.5, color: Rgba8::rgb(10, 20, 30) },
                ColorStop { position: 0.5, color: Rgba8::rgb(0, 0, 255) },
                ColorStop { position: 1.0, color: Rgba8::WHITE },
            ]
        }
        .sample(0.4999999)
    );
    assert_eq!(
        normalized.sample(0.5000001),
        ColorRamp::Custom {
            stops: vec![
                ColorStop { position: 0.0, color: Rgba8::BLACK },
                ColorStop { position: 0.5, color: Rgba8::rgb(255, 0, 0) },
                ColorStop { position: 0.5, color: Rgba8::rgb(10, 20, 30) },
                ColorStop { position: 0.5, color: Rgba8::rgb(0, 0, 255) },
                ColorStop { position: 1.0, color: Rgba8::WHITE },
            ]
        }
        .sample(0.5000001)
    );
    if let ColorRamp::Custom { stops: kept } = normalized {
        assert_eq!(kept.len(), 4, "middle redundant stop at 0.5 should be deduped");
    }
    Ok(())
}

#[test]
fn r1_s01_07_custom_ramp_cost() {
    let n = 20_000usize;
    let stops: Vec<ColorStop> =
        (0..n).rev().map(|i| ColorStop { position: (i as f64) / ((n - 1) as f64), color: Rgba8::rgb((i % 256) as u8, 128, 64) }).collect();
    let ramp = ColorRamp::Custom { stops };
    let t0 = Instant::now();
    for k in 0..200 {
        let _ = std::hint::black_box(ramp.sample((k as f64) / 200.0));
    }
    let per = t0.elapsed().as_secs_f64() / 200.0;
    assert!(per < 20e-6, "per sample took {per:?}");
}

#[test]
fn r1_s01_07_custom_constructor_validation_and_bounds() {
    let too_many: Vec<ColorStop> =
        (0..=MAX_CUSTOM_STOPS).map(|i| ColorStop { position: (i as f64) / (MAX_CUSTOM_STOPS as f64), color: Rgba8::BLACK }).collect();
    assert_eq!(ColorRamp::custom(too_many.clone()), Err(ColorRampError::TooManyStops { count: MAX_CUSTOM_STOPS + 1, max: MAX_CUSTOM_STOPS }));
    assert_eq!(
        ColorRamp::Custom { stops: too_many }.validate(),
        Err(ColorRampError::TooManyStops { count: MAX_CUSTOM_STOPS + 1, max: MAX_CUSTOM_STOPS })
    );

    let non_finite = vec![ColorStop { position: 0.0, color: Rgba8::BLACK }, ColorStop { position: f64::NAN, color: Rgba8::WHITE }];
    assert_eq!(ColorRamp::custom(non_finite), Err(ColorRampError::NonFinitePosition { index: 1 }));
}

struct BombCustomRampDeserializer {
    count: usize,
    reported_hint: usize,
}

impl<'de> Deserializer<'de> for BombCustomRampDeserializer {
    type Error = DeValueError;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("unsupported"))
    }

    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_enum(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

impl<'de> EnumAccess<'de> for BombCustomRampDeserializer {
    type Error = DeValueError;
    type Variant = Self;

    fn variant_seed<V: serde::de::DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant = seed.deserialize(serde::de::value::StrDeserializer::<DeValueError>::new("Custom"))?;
        Ok((variant, self))
    }
}

impl<'de> VariantAccess<'de> for BombCustomRampDeserializer {
    type Error = DeValueError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Err(serde::de::Error::custom("expected struct variant"))
    }
    fn newtype_variant_seed<T: serde::de::DeserializeSeed<'de>>(self, _seed: T) -> Result<T::Value, Self::Error> {
        Err(serde::de::Error::custom("expected struct variant"))
    }
    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("expected struct variant"))
    }
    fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(CustomFieldsMap { count: self.count, reported_hint: self.reported_hint, done: false })
    }
}

struct CustomFieldsMap {
    count: usize,
    reported_hint: usize,
    done: bool,
}

impl<'de> MapAccess<'de> for CustomFieldsMap {
    type Error = DeValueError;

    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        let k = seed.deserialize(serde::de::value::StrDeserializer::<DeValueError>::new("stops"))?;
        Ok(Some(k))
    }

    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        seed.deserialize(StopsSeqDe { remaining: self.count, idx: 0, reported_hint: self.reported_hint })
    }
}

struct StopsSeqDe {
    remaining: usize,
    idx: usize,
    reported_hint: usize,
}

impl<'de> Deserializer<'de> for StopsSeqDe {
    type Error = DeValueError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_seq(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de> SeqAccess<'de> for StopsSeqDe {
    type Error = DeValueError;

    fn size_hint(&self) -> Option<usize> {
        Some(self.reported_hint)
    }

    fn next_element_seed<T: serde::de::DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error> {
        if self.idx >= self.remaining {
            return Ok(None);
        }
        assert!(self.idx <= MAX_CUSTOM_STOPS + 1, "deserializer kept reading past MAX_CUSTOM_STOPS!");
        let val = seed.deserialize(SingleStopDe { position: (self.idx as f64) / ((MAX_CUSTOM_STOPS + 1) as f64) })?;
        self.idx += 1;
        Ok(Some(val))
    }
}

struct SingleStopDe {
    position: f64,
}

impl<'de> Deserializer<'de> for SingleStopDe {
    type Error = DeValueError;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("expected struct"))
    }

    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(StopMap { step: 0, position: self.position })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

struct StopMap {
    step: u8,
    position: f64,
}

impl<'de> MapAccess<'de> for StopMap {
    type Error = DeValueError;

    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        let key = match self.step {
            0 => "position",
            1 => "color",
            _ => return Ok(None),
        };
        Ok(Some(seed.deserialize(serde::de::value::StrDeserializer::<DeValueError>::new(key))?))
    }

    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        self.step += 1;
        match self.step {
            1 => seed.deserialize(serde::de::value::F64Deserializer::<DeValueError>::new(self.position)),
            _ => seed.deserialize(RgbaMapDe),
        }
    }
}

struct RgbaMapDe;

impl<'de> Deserializer<'de> for RgbaMapDe {
    type Error = DeValueError;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom("expected Rgba8 struct"))
    }

    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(RgbaMap { step: 0 })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

struct RgbaMap {
    step: u8,
}

impl<'de> MapAccess<'de> for RgbaMap {
    type Error = DeValueError;

    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        let key = match self.step {
            0 => "r",
            1 => "g",
            2 => "b",
            3 => "a",
            _ => return Ok(None),
        };
        Ok(Some(seed.deserialize(serde::de::value::StrDeserializer::<DeValueError>::new(key))?))
    }

    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        self.step += 1;
        seed.deserialize(serde::de::value::U8Deserializer::<DeValueError>::new(255))
    }
}

#[test]
fn r1_s01_07_deserializer_stops_before_excessive_allocation() {
    let err = ColorRamp::deserialize(BombCustomRampDeserializer { count: MAX_CUSTOM_STOPS + 100, reported_hint: 100_000_000 });
    assert!(err.is_err(), "expected TooManyStops rejection during bounded deserialization");

    let res = ColorRamp::deserialize(BombCustomRampDeserializer { count: MAX_CUSTOM_STOPS, reported_hint: 100_000_000 });
    assert!(res.is_ok(), "256 stops should deserialize cleanly even with inflated size_hint: {res:?}");
    if let Ok(ok) = res {
        assert_eq!(ok.validate(), Ok(()));
    }
}

#[test]
fn r1_s01_07_in_place_stop_mutation_invalidates_large_ramp_cache() {
    let stops: Vec<ColorStop> =
        (0..128).map(|i| ColorStop { position: (i as f64) / 127.0, color: if i < 64 { Rgba8::BLACK } else { Rgba8::WHITE } }).collect();
    let mut ramp = ColorRamp::Custom { stops };
    let before = ramp.sample(0.25);
    assert_eq!(before, Rgba8::BLACK);

    if let ColorRamp::Custom { ref mut stops } = ramp {
        for s in stops.iter_mut() {
            s.color = Rgba8::rgb(200, 10, 20);
        }
    }
    assert_eq!(ramp.sample(0.25), Rgba8::rgb(200, 10, 20));
}
