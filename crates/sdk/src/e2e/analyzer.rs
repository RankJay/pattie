//! Finds each received frame among the streams sent, and checks their order.
//!
//! A stream is what one converter run produced: on exact paths the replies
//! between two interrupts, frame for frame; where the SDK resamples, the same
//! `Converter` run offline over the same bytes. The received audio is split into
//! segments: a run of one stream's consecutive frames, silence, or frames found
//! nowhere. A frame is found from up to four consecutive frames that match a
//! place in one stream, so a cut, a drop, a duplicate or a torn frame shows as a
//! new run starting somewhere unexpected, or as unknown.

use std::cell::OnceCell;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Segment {
    /// `len` frames of stream `stream` from its frame `start`, received from
    /// frame `at` on.
    Run {
        stream: usize,
        start: usize,
        len: usize,
        at: usize,
    },
    /// `len` frames of exact silence from `at` on.
    Silence { len: usize, at: usize },
    /// `len` frames found in no stream, from `at` on.
    Unknown { len: usize, at: usize },
}

pub(super) struct Analyzer<'a> {
    channels: usize,
    streams: &'a [Vec<f32>],
    /// Every (stream, frame) by its first sample's bits: built only when a
    /// frame isn't the start of a later stream (a cut, a drop, a duplicate).
    by_first: OnceCell<HashMap<u32, Vec<(usize, usize)>>>,
}

fn same(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

fn silent(frame: &[f32]) -> bool {
    frame.iter().all(|&s| s == 0.0)
}

impl<'a> Analyzer<'a> {
    pub(super) fn new(channels: usize, streams: &'a [Vec<f32>]) -> Analyzer<'a> {
        Analyzer {
            channels,
            streams,
            by_first: OnceCell::new(),
        }
    }

    fn by_first(&self) -> &HashMap<u32, Vec<(usize, usize)>> {
        self.by_first.get_or_init(|| {
            let mut by_first: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
            for (s, stream) in self.streams.iter().enumerate() {
                for (n, frame) in stream.chunks_exact(self.channels).enumerate() {
                    by_first.entry(frame[0].to_bits()).or_default().push((s, n));
                }
            }
            by_first
        })
    }

    fn frames(&self, stream: usize) -> usize {
        self.streams[stream].len() / self.channels
    }

    fn sent(&self, stream: usize, n: usize) -> &[f32] {
        &self.streams[stream][n * self.channels..(n + 1) * self.channels]
    }

    /// Where `received`'s frame `i` was sent from: the one place up to four
    /// consecutive frames match (fewer where a run is shorter, or crosses into
    /// another one). Among several (a mono frame is only 16 bits): the start of
    /// the first stream after `last`, since streams come in order and a
    /// resampled stream's first frames depend only on its first input sample
    /// (μ-law has 254 values); else the nearest ahead of `next` in its stream,
    /// as after a drop.
    fn locate(
        &self,
        received: &[f32],
        i: usize,
        next: Option<(usize, usize)>,
        last: Option<usize>,
    ) -> Option<(usize, usize)> {
        let ch = self.channels;
        let total = received.len() / ch;
        let got = |k: usize| &received[k * ch..(k + 1) * ch];
        let avail = (i..total.min(i + 4))
            .take_while(|&k| !silent(got(k)))
            .count();
        // Usually the start of a later stream: the one that matches longest
        // (two replies can open alike: a resampler's first frames come from one
        // input sample).
        let starts = (last.map_or(0, |l| l + 1)..self.streams.len())
            .map(|s| (self.matching(received, i, s, 0), s))
            .filter(|&(len, _)| len >= avail.max(1));
        if let Some((_, s)) = starts.max_by_key(|&(len, s)| (len, std::cmp::Reverse(s))) {
            return Some((s, 0));
        }
        let candidates = self.by_first().get(&got(i)[0].to_bits())?;
        for w in (1..=avail).rev() {
            let fits: Vec<(usize, usize)> = candidates
                .iter()
                .copied()
                .filter(|&(s, n)| {
                    n + w <= self.frames(s) && (0..w).all(|k| same(got(i + k), self.sent(s, n + k)))
                })
                .collect();
            match fits.as_slice() {
                [] => {}
                [one] => return Some(*one),
                several => {
                    let start = several
                        .iter()
                        .copied()
                        .filter(|&(s, n)| n == 0 && last.is_none_or(|l| s > l))
                        .min();
                    return start.or_else(|| {
                        several
                            .iter()
                            .copied()
                            .filter(|&(s, n)| next.is_some_and(|(ns, nn)| s == ns && n >= nn))
                            .min()
                    });
                }
            }
        }
        None
    }

    /// How many frames from `received`'s frame `i` match stream `s` from its
    /// frame `n`, up to 4,096.
    fn matching(&self, received: &[f32], i: usize, s: usize, n: usize) -> usize {
        let ch = self.channels;
        let total = received.len() / ch;
        (0..4_096)
            .take_while(|&k| {
                i + k < total
                    && n + k < self.frames(s)
                    && same(
                        &received[(i + k) * ch..(i + k + 1) * ch],
                        self.sent(s, n + k),
                    )
            })
            .count()
    }

    pub(super) fn segments(&self, received: &[f32]) -> Vec<Segment> {
        let ch = self.channels;
        let mut out: Vec<Segment> = Vec::new();
        let mut next: Option<(usize, usize)> = None;
        let mut last: Option<usize> = None;
        for i in 0..received.len() / ch {
            let frame = &received[i * ch..(i + 1) * ch];
            let expected =
                next.filter(|&(s, n)| n < self.frames(s) && same(frame, self.sent(s, n)));
            let found = if expected.is_some() {
                expected
            } else if silent(frame) {
                None
            } else {
                self.locate(received, i, next, last)
            };
            let segment = match found {
                Some((s, n)) => {
                    next = Some((s, n + 1));
                    last = Some(s);
                    Segment::Run {
                        stream: s,
                        start: n,
                        len: 1,
                        at: i,
                    }
                }
                None if silent(frame) => Segment::Silence { len: 1, at: i },
                None => {
                    next = None;
                    Segment::Unknown { len: 1, at: i }
                }
            };
            self.reclaim_leading_silence(&mut out, segment);
            extend(&mut out, segment);
        }
        out
    }

    /// A stream found starting at frame `n` > 0 whose frames before `n` were
    /// sent as exact silence (a fresh resampler's first outputs can be 0.0):
    /// those frames arrived as the silence just before, so they're the stream's.
    fn reclaim_leading_silence(&self, out: &mut Vec<Segment>, segment: Segment) {
        let Segment::Run {
            stream, start, at, ..
        } = segment
        else {
            return;
        };
        let Some(&Segment::Silence { len, at: from }) = out.last() else {
            return;
        };
        if start == 0
            || len < start
            || from + len != at
            || !(0..start).all(|n| silent(self.sent(stream, n)))
        {
            return;
        }
        out.pop();
        if len > start {
            out.push(Segment::Silence {
                len: len - start,
                at: from,
            });
        }
        out.push(Segment::Run {
            stream,
            start: 0,
            len: start,
            at: at - start,
        });
    }
}

/// Appends a one-frame segment, joining it to the last one where it continues it.
fn extend(out: &mut Vec<Segment>, segment: Segment) {
    if let Some(last) = out.last_mut() {
        match (last, segment) {
            (
                Segment::Run {
                    stream,
                    start,
                    len,
                    at,
                },
                Segment::Run {
                    stream: s,
                    start: n,
                    at: i,
                    ..
                },
            ) if *stream == s && *start + *len == n && *at + *len == i => {
                *len += 1;
                return;
            }
            (Segment::Silence { len, .. }, Segment::Silence { .. })
            | (Segment::Unknown { len, .. }, Segment::Unknown { .. }) => {
                *len += 1;
                return;
            }
            _ => {}
        }
    }
    out.push(segment);
}

/// One stream as received: its runs, and the silences inside it (between two
/// of its runs) as (stream frame where it resumed, frames).
#[derive(Default, Debug)]
pub(super) struct Received {
    pub(super) runs: Vec<(usize, usize, usize)>,
    pub(super) inner_silence: Vec<(usize, usize)>,
    /// Frames jumped over (where `check_order` was told drops are allowed).
    pub(super) skipped: usize,
}

impl Received {
    pub(super) fn frames(&self) -> usize {
        self.runs.iter().map(|&(_, len, _)| len).sum()
    }

    /// Frames received before received frame `mark`.
    pub(super) fn before(&self, mark: usize) -> usize {
        self.runs
            .iter()
            .map(|&(_, len, at)| mark.saturating_sub(at).min(len))
            .sum()
    }
}

/// Checks the general invariants: nothing unknown; streams in order, never
/// interleaved; each starts at its first frame and continues frame by frame (no
/// drop, duplicate or reordering), silence allowed between its runs. Returns
/// each stream as received.
pub(super) fn check_order(segments: &[Segment], streams: usize) -> Result<Vec<Received>, String> {
    check(segments, streams, false)
}

/// As `check_order`, but a stream may jump forward (frames dropped, counted in
/// `skipped`), never back.
pub(super) fn check_order_with_drops(
    segments: &[Segment],
    streams: usize,
) -> Result<Vec<Received>, String> {
    check(segments, streams, true)
}

fn check(segments: &[Segment], streams: usize, drops: bool) -> Result<Vec<Received>, String> {
    let mut received: Vec<Received> = (0..streams).map(|_| Received::default()).collect();
    let mut current: Option<usize> = None;
    let mut silence_since: Option<usize> = None;
    for &segment in segments {
        match segment {
            Segment::Unknown { len, at } => {
                return Err(format!(
                    "{len} frame(s) at received frame {at} match nothing sent"
                ));
            }
            Segment::Silence { len, .. } => silence_since = Some(silence_since.unwrap_or(0) + len),
            Segment::Run {
                stream,
                start,
                len,
                at,
            } => {
                if current.is_some_and(|c| stream < c) {
                    return Err(format!(
                        "stream {stream} frame {start} at received frame {at}, after stream {current:?} began"
                    ));
                }
                let r = &mut received[stream];
                let expected = r.runs.last().map_or(0, |&(s, l, _)| s + l);
                if drops && start > expected {
                    r.skipped += start - expected;
                } else if start != expected {
                    return Err(format!(
                        "stream {stream} jumps to frame {start} at received frame {at}, expected {expected}"
                    ));
                }
                if current == Some(stream)
                    && let Some(gap) = silence_since
                {
                    r.inner_silence.push((start, gap));
                }
                r.runs.push((start, len, at));
                current = Some(stream);
                silence_since = None;
            }
        }
    }
    Ok(received)
}
