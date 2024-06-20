use core::ops::{AddAssign, Div, Mul, Sub};

const HISTORY_DEPTH: usize = 16;

pub struct StatsBuffer<T>(heapless::HistoryBuffer<T, HISTORY_DEPTH>);

impl<T> StatsBuffer<T> {
    pub fn write(&mut self, t: T) {
        self.0.write(t)
    }
}

impl<T> Default for StatsBuffer<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> StatsBuffer<T>
where
    T: Clone + Default,
    T: Div<usize, Output = T>,
    T: AddAssign,
{
    pub fn mean(&self) -> T {
        let len = self.0.len();

        let mut res = T::default();
        for x in self.0.iter().cloned() {
            res += x;
        }

        res / len
    }
}

impl<T> StatsBuffer<T>
where
    T: Clone + Default,
    T: Div<usize, Output = T>,
    T: AddAssign,
    T: Sub<Output = T>,
    T: Mul<Output = T>,
{
    pub fn variance(&self) -> T {
        let mean = self.mean();
        let len = self.0.len();

        let mut res = T::default();
        for x in self.0.iter().cloned() {
            let a: T = mean.clone() - x;
            res += a.clone() * a;
        }

        res / len
    }
}
