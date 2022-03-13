//! The fuzzer, and state are the core pieces of every good fuzzer

use alloc::rc::Rc;
use core::{cell::RefCell, fmt::Debug, marker::PhantomData, ops::Deref, time::Duration};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
#[cfg(feature = "std")]
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    bolts::{
        rands::Rand,
        serdeany::{SerdeAny, SerdeAnyMap},
    },
    corpus::Corpus,
    events::{Event, EventFirer, LogSeverity},
    feedbacks::{Feedback, FeedbackState},
    fuzzer::{Evaluator, ExecuteInputResult},
    generators::Generator,
    inputs::Input,
    monitors::ClientPerfMonitor,
    Error,
};

/// The maximum size of a testcase
pub const DEFAULT_MAX_SIZE: usize = 1_048_576;

/// The [`State`] of the fuzzer.
/// Contains all important information about the current run.
/// Will be used to restart the fuzzing process at any timme.
pub trait State: Serialize + DeserializeOwned {}

/// Trait for elements offering a corpus
pub trait HasCorpus<I: Input> {
    /// The associated type implementing [`Corpus`].
    type Corpus: Corpus<I>;
    /// The testcase corpus
    fn corpus(&self) -> &Self::Corpus;
    /// The testcase corpus (mutable)
    fn corpus_mut(&mut self) -> &mut Self::Corpus;
}

/// Interact with the maximum size
pub trait HasMaxSize {
    /// The maximum size hint for items and mutations returned
    fn max_size(&self) -> usize;
    /// Sets the maximum size hint for the items and mutations
    fn set_max_size(&mut self, max_size: usize);
}

/// Trait for elements offering a corpus of solutions
pub trait HasSolutions<I: Input> {
    /// The associated type implementing [`Corpus`] for solutions
    type Solutions: Corpus<I>;
    /// The solutions corpus
    fn solutions(&self) -> &Self::Solutions;
    /// The solutions corpus (mutable)
    fn solutions_mut(&mut self) -> &mut Self::Solutions;
}

/// Trait for elements offering a rand
pub trait HasRand {
    /// The associated type implementing [`Rand`]
    type Rand: Rand;
    /// The rand instance
    fn rand(&self) -> &Self::Rand;
    /// The rand instance (mutable)
    fn rand_mut(&mut self) -> &mut Self::Rand;
}

/// Trait for offering a [`ClientPerfMonitor`]
pub trait HasClientPerfMonitor {
    /// [`ClientPerfMonitor`] itself
    fn introspection_monitor(&self) -> &ClientPerfMonitor;

    /// Mutatable ref to [`ClientPerfMonitor`]
    fn introspection_monitor_mut(&mut self) -> &mut ClientPerfMonitor;

    /// This node's stability
    fn stability(&self) -> &Option<f32>;

    /// This node's stability (mutable)
    fn stability_mut(&mut self) -> &mut Option<f32>;
}

/// Trait for elements offering metadata
pub trait HasMetadata {
    /// A map, storing all metadata
    fn metadata(&self) -> &SerdeAnyMap;
    /// A map, storing all metadata (mutable)
    fn metadata_mut(&mut self) -> &mut SerdeAnyMap;

    /// Add a metadata to the metadata map
    #[inline]
    fn add_metadata<M>(&mut self, meta: M)
    where
        M: SerdeAny,
    {
        self.metadata_mut().insert(meta);
    }

    /// Check for a metadata
    #[inline]
    fn has_metadata<M>(&self) -> bool
    where
        M: SerdeAny,
    {
        self.metadata().get::<M>().is_some()
    }
}

/// Trait for elements offering feedback and objective states
pub trait HasFeedbackObjectiveStates {
    /// The state tree for feedbacks, stored in the first part of the feedback state tuple
    type FeedbackState: FeedbackState;
    /// The state tree for objective feedbacks (2nd part of the tuple)
    type ObjectiveState: FeedbackState;

    /// The feedback states and objective state tuple, as borrow-able [`Rc`]
    /// Using `Rc` allows us to keep the actual data in the [`State`], so it can be serialized in one go.
    fn feedback_objective_states(&self)
        -> Rc<RefCell<(Self::FeedbackState, Self::ObjectiveState)>>;
}

/// Trait for the execution counter
pub trait HasExecutions {
    /// The executions counter
    fn executions(&self) -> &usize;

    /// The executions counter (mutable)
    fn executions_mut(&mut self) -> &mut usize;
}

/// Trait for the starting time
pub trait HasStartTime {
    /// The starting time
    fn start_time(&self) -> &Duration;

    /// The starting time (mutable)
    fn start_time_mut(&mut self) -> &mut Duration;
}

/// The state a fuzz run.
#[derive(Serialize, Deserialize, Debug)]
#[serde(bound = "FS: serde::de::DeserializeOwned, OS: serde::de::DeserializeOwned")]
pub struct StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    /// RNG instance
    rand: R,
    /// How many times the executor ran the harness/target
    executions: usize,
    /// At what time the fuzzing started
    start_time: Duration,
    /// The corpus
    corpus: C,
    /// States of the feedback, and objectives used to evaluate an input
    /// This is a [`Rc`] type to allow feedback states to be passed fo functons
    feedback_objective_states: Rc<RefCell<(FS, OS)>>,
    // Solutions corpus
    solutions: SC,
    /// Metadata stored for this state by one of the components
    metadata: SerdeAnyMap,
    /// MaxSize testcase size for mutators that appreciate it
    max_size: usize,
    /// The stability of the current fuzzing process
    stability: Option<f32>,

    /// Performance statistics for this fuzzer
    #[cfg(feature = "introspection")]
    introspection_monitor: ClientPerfMonitor,

    phantom: PhantomData<I>,
}

impl<C, FS, I, OS, R, SC> Clone for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I> + Clone,
    I: Input,
    R: Rand + Clone,
    FS: FeedbackState + Clone,
    OS: FeedbackState + Clone,
    SC: Corpus<I> + Clone,
{
    fn clone(&self) -> Self {
        let (feedback_state, objective_state) = self.feedback_objective_states.borrow().deref();
        Self {
            rand: self.rand.clone(),
            executions: self.executions,
            start_time: self.start_time.clone(),
            corpus: self.corpus.clone(),
            // make sure we clone the actual state instead of just the reference
            feedback_objective_states: Rc::new(RefCell::new((
                feedback_state.clone(),
                objective_state.clone(),
            ))),
            solutions: self.solutions.clone(),
            metadata: self.metadata.clone(),
            max_size: self.max_size,
            stability: self.stability.clone(),
            #[cfg(feature = "introspection")]
            introspection_monitor: self.introspection_monitor.clone(),
            phantom: PhantomData,
        }
    }
}

impl<C, FS, I, OS, R, SC> State for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
}

impl<C, FS, I, OS, R, SC> HasRand for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    type Rand = R;

    /// The rand instance
    #[inline]
    fn rand(&self) -> &Self::Rand {
        &self.rand
    }

    /// The rand instance (mutable)
    #[inline]
    fn rand_mut(&mut self) -> &mut Self::Rand {
        &mut self.rand
    }
}

impl<C, FS, I, OS, R, SC> HasCorpus<I> for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    type Corpus = C;

    /// Returns the corpus
    #[inline]
    fn corpus(&self) -> &C {
        &self.corpus
    }

    /// Returns the mutable corpus
    #[inline]
    fn corpus_mut(&mut self) -> &mut C {
        &mut self.corpus
    }
}

impl<C, FS, I, OS, R, SC> HasSolutions<I> for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    type Solutions = SC;

    /// Returns the solutions corpus
    #[inline]
    fn solutions(&self) -> &SC {
        &self.solutions
    }

    /// Returns the solutions corpus (mutable)
    #[inline]
    fn solutions_mut(&mut self) -> &mut SC {
        &mut self.solutions
    }
}

impl<C, FS, I, OS, R, SC> HasMetadata for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    /// Get all the metadata into an [`hashbrown::HashMap`]
    #[inline]
    fn metadata(&self) -> &SerdeAnyMap {
        &self.metadata
    }

    /// Get all the metadata into an [`hashbrown::HashMap`] (mutable)
    #[inline]
    fn metadata_mut(&mut self) -> &mut SerdeAnyMap {
        &mut self.metadata
    }
}

impl<C, FS, I, OS, R, SC> HasFeedbackObjectiveStates for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    type FeedbackState = FS;

    type ObjectiveState = OS;

    /// The feedback and objective states
    #[inline]
    fn feedback_objective_states(&self) -> Rc<RefCell<(FS, OS)>> {
        self.feedback_objective_states.clone()
    }
}

impl<C, FS, I, OS, R, SC> HasExecutions for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    /// The executions counter
    #[inline]
    fn executions(&self) -> &usize {
        &self.executions
    }

    /// The executions counter (mutable)
    #[inline]
    fn executions_mut(&mut self) -> &mut usize {
        &mut self.executions
    }
}

impl<C, FS, I, OS, R, SC> HasMaxSize for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    fn max_size(&self) -> usize {
        self.max_size
    }

    fn set_max_size(&mut self, max_size: usize) {
        self.max_size = max_size;
    }
}

impl<C, FS, I, OS, R, SC> HasStartTime for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    /// The starting time
    #[inline]
    fn start_time(&self) -> &Duration {
        &self.start_time
    }

    /// The starting time (mutable)
    #[inline]
    fn start_time_mut(&mut self) -> &mut Duration {
        &mut self.start_time
    }
}

#[cfg(feature = "std")]
impl<C, FS, I, OS, R, SC> StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    /// Loads inputs from a directory.
    /// If `forced` is `true`, the value will be loaded,
    /// even if it's not considered to be `interesting`.
    pub fn load_from_directory<E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        manager: &mut EM,
        in_dir: &Path,
        forced: bool,
        loader: &mut dyn FnMut(&mut Z, &mut Self, &Path) -> Result<I, Error>,
    ) -> Result<(), Error>
    where
        Z: Evaluator<E, EM, I, Self>,
    {
        for entry in fs::read_dir(in_dir)? {
            let entry = entry?;
            let path = entry.path();
            let attributes = fs::metadata(&path);

            if attributes.is_err() {
                continue;
            }

            let attr = attributes?;

            if attr.is_file() && attr.len() > 0 {
                println!("Loading file {:?} ...", &path);
                let input = loader(fuzzer, self, &path)?;
                if forced {
                    let _ = fuzzer.add_input(self, executor, manager, input)?;
                } else {
                    let (res, _) = fuzzer.evaluate_input(self, executor, manager, input)?;
                    if res == ExecuteInputResult::None {
                        println!("File {:?} was not interesting, skipped.", &path);
                    }
                }
            } else if attr.is_dir() {
                self.load_from_directory(fuzzer, executor, manager, &path, forced, loader)?;
            }
        }

        Ok(())
    }

    /// Loads initial inputs from the passed-in `in_dirs`.
    /// If `forced` is true, will add all testcases, no matter what.
    fn load_initial_inputs_internal<E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        manager: &mut EM,
        in_dirs: &[PathBuf],
        forced: bool,
    ) -> Result<(), Error>
    where
        Z: Evaluator<E, EM, I, Self>,
        EM: EventFirer<I>,
    {
        for in_dir in in_dirs {
            self.load_from_directory(
                fuzzer,
                executor,
                manager,
                in_dir,
                forced,
                &mut |_, _, path| I::from_file(&path),
            )?;
        }
        manager.fire(
            self,
            Event::Log {
                severity_level: LogSeverity::Debug,
                message: format!("Loaded {} initial testcases.", self.corpus().count()), // get corpus count
                phantom: PhantomData,
            },
        )?;
        Ok(())
    }

    /// Loads all intial inputs, even if they are not considered `interesting`.
    /// This is rarely the right method, use `load_initial_inputs`,
    /// and potentially fix your `Feedback`, instead.
    pub fn load_initial_inputs_forced<E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        manager: &mut EM,
        in_dirs: &[PathBuf],
    ) -> Result<(), Error>
    where
        Z: Evaluator<E, EM, I, Self>,
        EM: EventFirer<I>,
    {
        self.load_initial_inputs_internal(fuzzer, executor, manager, in_dirs, true)
    }

    /// Loads initial inputs from the passed-in `in_dirs`.
    pub fn load_initial_inputs<E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        manager: &mut EM,
        in_dirs: &[PathBuf],
    ) -> Result<(), Error>
    where
        Z: Evaluator<E, EM, I, Self>,
        EM: EventFirer<I>,
    {
        self.load_initial_inputs_internal(fuzzer, executor, manager, in_dirs, false)
    }
}

impl<C, FS, I, OS, R, SC> StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    fn generate_initial_internal<G, E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        generator: &mut G,
        manager: &mut EM,
        num: usize,
        forced: bool,
    ) -> Result<(), Error>
    where
        G: Generator<I, Self>,
        Z: Evaluator<E, EM, I, Self>,
        EM: EventFirer<I>,
    {
        let mut added = 0;
        for _ in 0..num {
            let input = generator.generate(self)?;
            if forced {
                let _ = fuzzer.add_input(self, executor, manager, input)?;
                added += 1;
            } else {
                let (res, _) = fuzzer.evaluate_input(self, executor, manager, input)?;
                if res != ExecuteInputResult::None {
                    added += 1;
                }
            }
        }
        manager.fire(
            self,
            Event::Log {
                severity_level: LogSeverity::Debug,
                message: format!("Loaded {} over {} initial testcases", added, num),
                phantom: PhantomData,
            },
        )?;
        Ok(())
    }

    /// Generate `num` initial inputs, using the passed-in generator and force the addition to corpus.
    pub fn generate_initial_inputs_forced<G, E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        generator: &mut G,
        manager: &mut EM,
        num: usize,
    ) -> Result<(), Error>
    where
        G: Generator<I, Self>,
        Z: Evaluator<E, EM, I, Self>,
        EM: EventFirer<I>,
    {
        self.generate_initial_internal(fuzzer, executor, generator, manager, num, true)
    }

    /// Generate `num` initial inputs, using the passed-in generator.
    pub fn generate_initial_inputs<G, E, EM, Z>(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        generator: &mut G,
        manager: &mut EM,
        num: usize,
    ) -> Result<(), Error>
    where
        G: Generator<I, Self>,
        Z: Evaluator<E, EM, I, Self>,
        EM: EventFirer<I>,
    {
        self.generate_initial_internal(fuzzer, executor, generator, manager, num, false)
    }

    /// Creates a new `State`, taking ownership of all of the individual components during fuzzing.
    pub fn new<F, O>(
        rand: R,
        corpus: C,
        solutions: SC,
        feedbacks: &mut F,
        objectives: &mut O,
    ) -> Result<Self, Error>
    where
        F: Feedback<I, Self, FeedbackState = FS>,
        O: Feedback<I, Self, FeedbackState = OS>,
    {
        Ok(Self {
            rand,
            executions: 0,
            stability: None,
            start_time: Duration::from_millis(0),
            metadata: SerdeAnyMap::default(),
            corpus,
            feedback_objective_states: Rc::new(RefCell::new((
                feedbacks.init_state()?,
                objectives.init_state()?,
            ))),
            solutions,
            max_size: DEFAULT_MAX_SIZE,
            #[cfg(feature = "introspection")]
            introspection_monitor: ClientPerfMonitor::new(),
            phantom: PhantomData,
        })
    }
}

#[cfg(feature = "introspection")]
impl<C, FS, I, OS, R, SC> HasClientPerfMonitor for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    fn introspection_monitor(&self) -> &ClientPerfMonitor {
        &self.introspection_monitor
    }

    fn introspection_monitor_mut(&mut self) -> &mut ClientPerfMonitor {
        &mut self.introspection_monitor
    }

    /// This node's stability
    #[inline]
    fn stability(&self) -> &Option<f32> {
        &self.stability
    }

    /// This node's stability (mutable)
    #[inline]
    fn stability_mut(&mut self) -> &mut Option<f32> {
        &mut self.stability
    }
}

#[cfg(not(feature = "introspection"))]
impl<C, FS, I, OS, R, SC> HasClientPerfMonitor for StdState<C, FS, I, OS, R, SC>
where
    C: Corpus<I>,
    I: Input,
    R: Rand,
    FS: FeedbackState,
    OS: FeedbackState,
    SC: Corpus<I>,
{
    fn introspection_monitor(&self) -> &ClientPerfMonitor {
        unimplemented!()
    }

    fn introspection_monitor_mut(&mut self) -> &mut ClientPerfMonitor {
        unimplemented!()
    }

    /// This node's stability
    #[inline]
    fn stability(&self) -> &Option<f32> {
        &self.stability
    }

    /// This node's stability (mutable)
    #[inline]
    fn stability_mut(&mut self) -> &mut Option<f32> {
        &mut self.stability
    }
}
