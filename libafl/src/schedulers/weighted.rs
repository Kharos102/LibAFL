//! The queue corpus scheduler with weighted queue item selection from aflpp (https://github.com/AFLplusplus/AFLplusplus/blob/1d4f1e48797c064ee71441ba555b29fc3f467983/src/afl-fuzz-queue.c#L32)
//! This queue corpus scheduler needs calibration stage and the power schedule stage.

use alloc::string::{String, ToString};

use crate::{
    bolts::rands::Rand,
    corpus::{Corpus, PowerScheduleTestcaseMetaData},
    inputs::Input,
    schedulers::{Scheduler, powersched::{PowerScheduleMetadata, PowerSchedule}},
    state::{HasCorpus, HasMetadata, HasRand},
    Error,
};
use core::marker::PhantomData;

crate::impl_serdeany!(WeightedScheduleMetadata);

/// The Metadata for `WeightedScheduler`
pub struct WeightedScheduleMetadata {
    /// The fuzzer execution spent in the current cycles
    runs_in_current_cycle: u64,
    /// Alias table for weighted queue entry selection
    alias_table: Vec<u32>,
    /// Probability for which queue entry is selected
    alias_probability: Vec<f64>,
    /// Cache the perf_score
    perf_scores: Vec<f64>,
}

impl WeightedScheduleMetadata {
    pub fn new() -> Self {
        Self {
            runs_in_current_cycle: 0,
            alias_table: vec![0],
            alias_probability: vec![0],
            perf_scores: vec![0],
        }
    }

    pub fn runs_in_current_cycle(&self) -> u64 {
        self.runs_in_current_cycle
    }

    pub fn set_runs_current_cycle(&mut self, cycles: u64) {
        self.runs_in_current_cycle = cycles;
    }

    pub fn alias_table(&self) -> &[u32] {
        &self.alias_table
    }

    pub fn set_alias_table(&mut self, table: Vec<u32>) {
        self.alias_table = table;
    }

    pub fn alias_probability(&self) -> &[f64] {
        &self.alias_probability
    }

    pub fn set_alias_probability(&mut self, probability: Vec<f64>) {
        self.alias_probability = probability;
    }

    pub fn perf_scores(&self) -> &[f64] {
        &self.perf_scores
    }

    pub fn set_perf_scores(&mut self, perf_scores: Vec<f64>) {
        self.perf_scores = perf_scores
    }
}

/// A corpus scheduler using power schedules with weighted queue item selection algo.
#[derive(Clone, Debug)]
pub struct WeightedScheduler<I, S> {
    phantom: PhantomData<(I, S)>
}

impl<I, S> Default for WeightedScheduler<I, S>
where
    I: Input,
    S: HasCorpus<I> + HasMetadata + HasRand,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<I, S> WeightedScheduler<I, S> 
where
    I: Input,
    S: HasCorpus<I> + HasMetadata + HasRand,
{
    /// Create a new [`PowerQueueScheduler`]
    #[must_use]
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }

    pub fn create_alias_table(&self, state: &mut S) -> Result<(), Error> 
    {
        let n = state.corpus().count();

        let alias_table : Vec<u32> = vec![0, n];
        let alias_probability: Vec<f64> = vec![0.0, n];
        let perf_scores: Vec<f64> = vec![0.0, n];

        let P : Vec<f64> = vec![0, n];
        let S : Vec<u32> = vec![0, n];
        let L : Vec<u32> = vec![0, n];

        let sum : f64 = 0.0;

        let psmeta = state
            .metadata_mut()
            .get_mut::<PowerScheduleMetadata>()
            .ok_or_else(|| {
                Error::KeyNotFound("PowerScheduleMetadata not found".to_string())
            })?;

        let fuzz_mu = if psmeta.strat() == PowerSchedule::COE {
            let corpus = state.corpus();
            let mut n_paths = 0;
            let mut v = 0.0;
            for idx in 0..corpus.count() {
                let n_fuzz_entry = corpus
                    .get(idx)?
                    .borrow()
                    .metadata()
                    .get::<PowerScheduleTestcaseMetaData>()
                    .ok_or_else(|| Error::KeyNotFound("PowerScheduleTestData not found".to_string()))?
                    .n_fuzz_entry();
                v += libm::log2(f64::from(psmeta.n_fuzz()[n_fuzz_entry]));
                n_paths += 1;
            }
    
            if n_paths == 0 {
                return Err(Error::Unknown(String::from("Queue state corrput")));
            }
    
            v /= f64::from(n_paths);
            v
        }
        else{
            0.0
        };

        for i in 0..n {
            let testcase = state.corpus().get(i)?.borrow_mut();
            let perf_score = testcase.calculate_score(psmeta, fuzz_mu);
            perf_scores[i] = perf_score;
            sum += perf_score;
        }

        for i in 0..n {
            P[i] = perf_scores[i] / n;
        }

        let nS: usize = 0;
        let nL: usize = 0;


        Ok()
    }
}



impl<I, S> Scheduler<I, S> for WeightedScheduler<I, S>
where
    S: HasCorpus<I> + HasMetadata + HasRand,
    I: Input,
{
    /// Add an entry to the corpus and return its index
    fn on_add(&self, state: &mut S, idx: usize) -> Result<(), Error> {
        let current_idx = *state.corpus().current();

        let mut depth = match current_idx {
            Some(parent_idx) => state
                .corpus()
                .get(parent_idx)?
                .borrow_mut()
                .metadata_mut()
                .get_mut::<PowerScheduleTestcaseMetaData>()
                .ok_or_else(|| Error::KeyNotFound("PowerScheduleTestData not found".to_string()))?
                .depth(),
            None => 0,
        };

        // Attach a `PowerScheduleTestData` to the queue entry.
        depth += 1;
        state
            .corpus()
            .get(idx)?
            .borrow_mut()
            .add_metadata(PowerScheduleTestcaseMetaData::new(depth));

        // Recrate the alias table
        self.create_alias_table(state);
        Ok(())
    }

    fn next(&self, state: &mut S) -> Result<usize, Error> {
        if state.corpus().count() == 0 {
            Err(Error::Empty(String::from("No entries in corpus")))
        } else {

            let wsmeta = state
                .metadata_mut()
                .get_mut::<WeightedScheduleMetadata>()
                .ok_or_else(|| {
                    Error::KeyNotFound("WeigthedScheduleMetadata not found".to_string())
                })?;

            if wsmeta.runs_in_current_cycle() > state.corpus().count() {
                // update depth

                let psmeta = state
                    .metadata_mut()
                    .get_mut::<PowerScheduleMetadata>()
                    .ok_or_else(|| {
                        Error::KeyNotFound("PowerScheduleMetadata not found".to_string())
                    })?;
                psmeta.set_queue_cycles(psmeta.queue_cycles() + 1);
                wsmeta.set_runs_current_cycle(0);
            }
            else{
                wsmeta.set_runs_current_cycle(wsmeta.runs_in_current_cycle());
            }



            let r = state.rand_mut().below(u64::MAX) as usize;
            let s = r % state.corpus().count();

            let idx = if r < wsmeta.alias_probability()[s] {
                s
            }
            else{
                wsmeta.alias_table()[s]
            };

            Ok(idx)
        }
    }
}
