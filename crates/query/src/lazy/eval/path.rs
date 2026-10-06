//! The steps of a path (`.a`, `.[3]`, `.[2:5]`, `.[]`), and the elements of a
//! long list by a number of threads.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use jsonquery_core::ValueKind;
use serde_json::Value;

use super::super::plan::{PathStep, Step};
use super::{Emission, Evaluator, Failure, Flow, Item, Lazy, Sink, Then};
use crate::to_val;

impl Evaluator<'_> {
    /// The steps of a path, from `lazy`: what each makes is taken through the
    /// steps after it, and what comes out of the last is given to `then`.
    pub(super) fn path<'t>(
        &self,
        steps: &[PathStep],
        lazy: Lazy<'t>,
        then: Then<'_, 't>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        let Some((first, rest)) = steps.split_first() else {
            return then(self, Item::Lazy(lazy), sink);
        };
        if self.stopped() {
            return Flow::Stop;
        }
        if first.step == Step::Each && self.worth_a_crowd(&lazy) {
            return self.each_by_many(rest, lazy, then, sink);
        }
        self.step(first, lazy, &mut |result| match result {
            Ok(Item::Lazy(next)) => self.path(rest, next, then, sink),
            // A value made by a step (a `null`, a gathered list of a slice) is
            // small: the steps after it are for jaq.
            Ok(Item::Value(value)) => self.path_of_value(rest, value, then, sink),
            Err(failure) => sink(failure.emission()),
        })
    }

    /// The rest of a path on a value in memory: what jaq makes of it.
    fn path_of_value<'t>(
        &self,
        steps: &[PathStep],
        value: Value,
        then: Then<'_, 't>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        if steps.is_empty() {
            return then(self, Item::Value(value), sink);
        }
        let text = format!(".{}", steps.iter().map(step_text).collect::<String>());
        let program = match self.program(&text) {
            Ok(program) => program,
            Err(e) => return sink(Emission::Error(e)),
        };
        for result in program.run(to_val(&value), Vec::new()) {
            let flow = match result {
                Ok(v) => then(self, Item::Value(v), sink),
                Err(e) => sink(Emission::Error(e)),
            };
            if flow == Flow::Stop {
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    // ---- the elements of a long list, by a number of threads -------------------

    /// Whether the elements of `lazy` are enough for threads to be worth
    /// starting: there are some, and they take some of the file.
    pub(super) fn worth_a_crowd(&self, lazy: &Lazy<'_>) -> bool {
        let tuning = self.limits.parallel;
        self.crowd
            && tuning.threads > 1
            && lazy.kind().is_container()
            && lazy.len() >= 2
            && lazy.bytes_up_to(tuning.min_bytes) >= tuning.min_bytes
    }

    /// The first elements of `lazy`, which this thread does: as many as the
    /// tuning says, or as take as many bytes. How many were done, and how many
    /// elements a thread is to take at a time of the rest, going by how big
    /// those were.
    pub(super) fn prefix<'t>(
        &self,
        lazy: &Lazy<'t>,
        each: &mut dyn FnMut(Lazy<'t>) -> Flow,
    ) -> Result<(usize, usize), Flow> {
        let tuning = self.limits.parallel;
        let (mut done, mut bytes) = (0usize, 0usize);
        for item in lazy.items_from(0) {
            if done >= tuning.head || bytes >= tuning.head_bytes {
                break;
            }
            bytes = bytes.saturating_add(item.bytes_up_to(usize::MAX).saturating_add(1));
            if each(item) == Flow::Stop || self.stopped() {
                return Err(Flow::Stop);
            }
            done += 1;
        }
        let average = (bytes / done.max(1)).max(1);
        let chunk = (tuning.chunk_bytes / average).clamp(1, tuning.chunk.max(1));
        Ok((done, chunk))
    }

    /// `work` on each element of `lazy`, what it makes handed to `deliver` in the
    /// order of the list: by this thread for a short list, and for a long or heavy
    /// one by threads, after the first few elements, which this thread does.
    pub(super) fn for_each_item<'t, R: Send>(
        &self,
        lazy: &Lazy<'t>,
        work: &(dyn Fn(&Evaluator<'_>, Lazy<'t>) -> R + Sync),
        deliver: &mut dyn FnMut(R) -> Flow,
    ) -> Flow {
        if !self.worth_a_crowd(lazy) {
            for item in lazy.items_from(0) {
                if self.stopped() || deliver(work(self, item)) == Flow::Stop {
                    return Flow::Stop;
                }
            }
            return Flow::Continue;
        }
        let (done, chunk) = match self.prefix(lazy, &mut |item| deliver(work(self, item))) {
            Ok(prefix) => prefix,
            Err(flow) => return flow,
        };
        if done >= lazy.len() {
            return Flow::Continue;
        }
        self.crowd(lazy, done, chunk, work, deliver)
    }

    /// `.[]` then `rest` then `then`, on the elements of `lazy`: the first few by
    /// this thread, which is all that a query that is after the first match
    /// needs, then runs of them by threads, each with an evaluator of its own.
    /// What the threads make is handed to `sink` in the order of the list.
    fn each_by_many<'t>(
        &self,
        rest: &[PathStep],
        lazy: Lazy<'t>,
        then: Then<'_, 't>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        self.for_each_item(
            &lazy,
            &|worker, item| {
                let mut made: Vec<Emission<'t>> = Vec::new();
                worker.path(rest, item, then, &mut |emission| {
                    made.push(emission);
                    Flow::Continue
                });
                made
            },
            &mut |made: Vec<Emission<'t>>| {
                for emission in made {
                    if sink(emission) == Flow::Stop {
                        return Flow::Stop;
                    }
                }
                Flow::Continue
            },
        )
    }

    /// The elements of `lazy` from the `head`th on, a run of `chunk` of them at a
    /// time by each of a number of threads: `work` is run on each, with an
    /// evaluator of the thread's own, and what it makes is handed to `deliver` in
    /// the order of the list, as if one thread had done it all. The threads do not
    /// get far ahead of what has been handed on, so what waits is a few runs.
    pub(super) fn crowd<'t, R: Send>(
        &self,
        lazy: &Lazy<'t>,
        head: usize,
        chunk: usize,
        work: &(dyn Fn(&Evaluator<'_>, Lazy<'t>) -> R + Sync),
        deliver: &mut dyn FnMut(R) -> Flow,
    ) -> Flow {
        let tuning = self.limits.parallel;
        let chunk = chunk.max(1);
        let len = lazy.len();
        let chunks = (len - head).div_ceil(chunk);
        // How far ahead of what has been handed on the threads may go.
        let window = tuning.threads * 4;
        let next = AtomicUsize::new(0);
        let taken = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let (sender, receiver) = crossbeam_channel::unbounded::<(usize, Vec<R>)>();

        let (cancelled, limits) = (self.cancelled, self.limits);
        std::thread::scope(|scope| {
            for _ in 0..tuning.threads {
                let sender = sender.clone();
                let (next, taken, stop) = (&next, &taken, &stop);
                scope.spawn(move || {
                    let worker = Evaluator::for_a_thread(cancelled, limits);
                    loop {
                        let c = next.fetch_add(1, Ordering::SeqCst);
                        if c >= chunks {
                            return;
                        }
                        while c >= taken.load(Ordering::Acquire) + window {
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_micros(100));
                        }
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let start = head + c * chunk;
                        let count = chunk.min(len - start);
                        let mut made = Vec::new();
                        for item in lazy.items_from(start).take(count) {
                            if stop.load(Ordering::Relaxed) || worker.stopped() {
                                break;
                            }
                            made.push(work(&worker, item));
                        }
                        if sender.send((c, made)).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(sender);

            let mut waiting: BTreeMap<usize, Vec<R>> = BTreeMap::new();
            let mut flow = Flow::Continue;
            'chunks: for want in 0..chunks {
                let made = loop {
                    if let Some(made) = waiting.remove(&want) {
                        break made;
                    }
                    match receiver.recv() {
                        Ok((c, made)) => {
                            waiting.insert(c, made);
                        }
                        // The threads are gone: what is left was not done.
                        Err(_) => break 'chunks,
                    }
                };
                for made in made {
                    if self.stopped() || deliver(made) == Flow::Stop {
                        flow = Flow::Stop;
                        break 'chunks;
                    }
                }
                taken.store(want + 1, Ordering::Release);
                if self.stopped() {
                    flow = Flow::Stop;
                    break;
                }
            }
            stop.store(true, Ordering::Relaxed);
            flow
        })
    }

    /// One step, on a node that is too big to be a value.
    fn step<'t>(
        &self,
        step: &PathStep,
        lazy: Lazy<'t>,
        emit: &mut dyn FnMut(Result<Item<'t>, Failure>) -> Flow,
    ) -> Flow {
        let kind = lazy.kind();

        let outcome: Result<Item<'t>, Failure> = match &step.step {
            Step::Key(key) => match kind {
                ValueKind::Object if lazy.is_whole() => Ok(match lazy.node().child_by_key(key) {
                    Some(child) => Item::Lazy(Lazy::of(child.node)),
                    None => Item::Value(Value::Null),
                }),
                ValueKind::Null => Ok(Item::Value(Value::Null)),
                _ => Err(Failure::Error(format!(
                    "cannot index {} with {}",
                    self.shown(&lazy),
                    serde_json::to_string(key).unwrap_or_default()
                ))),
            },
            Step::Index(index) => match kind {
                ValueKind::Array => {
                    let len = lazy.len() as i64;
                    let at = if *index < 0 { index + len } else { *index };
                    Ok(
                        match (0..len).contains(&at).then(|| lazy.item(at as usize)) {
                            Some(Some(item)) => Item::Lazy(item),
                            _ => Item::Value(Value::Null),
                        },
                    )
                }
                // jaq answers an object indexed by a number with `null`.
                ValueKind::Object | ValueKind::Null => Ok(Item::Value(Value::Null)),
                _ => Err(Failure::Error(format!(
                    "cannot index {} with {index}",
                    self.shown(&lazy)
                ))),
            },
            Step::Slice(from, to) => match kind {
                ValueKind::Array => {
                    let len = lazy.len() as i64;
                    let bound = |b: &Option<i64>, default: i64| match b {
                        None => default,
                        Some(b) if *b < 0 => (len + b).max(0),
                        Some(b) => (*b).min(len),
                    };
                    let (from, to) = (bound(from, 0), bound(to, len));
                    Ok(Item::Lazy(if from < to {
                        lazy.slice(from as usize, to as usize)
                    } else {
                        lazy.slice(0, 0)
                    }))
                }
                ValueKind::String => Err(Failure::Refusal(format!(
                    "cannot slice {}: a string that big is not sliced here",
                    self.describe(&lazy)
                ))),
                _ => Err(Failure::Error(format!(
                    "cannot use {} as rangeable (array or string)",
                    self.shown(&lazy)
                ))),
            },
            Step::Each => match kind {
                ValueKind::Array | ValueKind::Object => {
                    for item in lazy.items_from(0) {
                        if self.stopped() {
                            return Flow::Stop;
                        }
                        if emit(Ok(Item::Lazy(item))) == Flow::Stop {
                            return Flow::Stop;
                        }
                    }
                    return Flow::Continue;
                }
                _ => Err(Failure::Error(format!(
                    "cannot use {} as iterable (array or object)",
                    self.shown(&lazy)
                ))),
            },
        };

        match outcome {
            // `?`: what could not be done is no output; what is too big is.
            Err(Failure::Error(_)) if step.optional => Flow::Continue,
            result => emit(result),
        }
    }
}

/// A step as the text a jq program writes it as, after a `.`.
fn step_text(step: &PathStep) -> String {
    let mut text = match &step.step {
        Step::Key(key) => format!("[{}]", serde_json::to_string(key).unwrap_or_default()),
        Step::Index(index) => format!("[{index}]"),
        Step::Slice(from, to) => format!(
            "[{}:{}]",
            from.map(|n| n.to_string()).unwrap_or_default(),
            to.map(|n| n.to_string()).unwrap_or_default()
        ),
        Step::Each => "[]".to_owned(),
    };
    if step.optional {
        text.push('?');
    }
    text
}
