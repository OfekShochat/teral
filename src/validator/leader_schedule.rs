use rand::{
    distributions::WeightedIndex,
    prelude::{Distribution, StdRng},
    Rng, SeedableRng,
};

const SCHEDULE_SEED: u64 = 13409387784011516370;

// NOTE: weighted random done every epoch by a set of validators that we choose randomly based on the seed.
// the seed is somehow manipulated every epoch/block.

pub struct LeaderSchedule {
    curr_seed: u64,
    rng: StdRng,
}

impl LeaderSchedule {
    pub fn new() -> Self {
        Self {
            curr_seed: SCHEDULE_SEED,
            rng: StdRng::seed_from_u64(SCHEDULE_SEED),
        }
    }

    pub fn get_validator(&mut self) {
        let a = WeightedIndex::new([2, 1]).unwrap(); // somehow get the validator list and the stake distribution.
        a.sample(&mut self.rng);
        self.curr_seed = 0; // somehow manipulate the seed. maybe hash it with the chosen validator's pubkey?
        self.rng = StdRng::seed_from_u64(self.curr_seed);
    }
}
