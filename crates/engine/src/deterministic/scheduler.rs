//! Bounded virtual timer queue. Intervals re-arm before delivery, retaining their handle.
#[derive(Clone, Debug)]
pub struct Event<T> {
    pub id: u64,
    pub at: f64,
    pub sequence: u64,
    pub payload: T,
    interval: f64,
}

#[derive(Debug)]
pub struct Scheduler<T> {
    now: f64,
    sequence: u64,
    capacity: usize,
    events: Vec<Event<T>>,
}

fn delay(ms: f64) -> f64 {
    if !ms.is_finite() || ms <= 0.0 {
        0.0
    } else {
        ms.min(2147483647.0)
    }
}

impl<T: Clone> Scheduler<T> {
    pub fn new(capacity: usize) -> Result<Self, &'static str> {
        if capacity == 0 {
            return Err("Timer capacity must be positive");
        }
        Ok(Self {
            now: 0.0,
            sequence: 0,
            capacity,
            events: Vec::new(),
        })
    }
    pub fn now(&self) -> f64 {
        self.now
    }
    pub fn pending(&self) -> usize {
        self.events.len()
    }
    pub fn next_at(&self) -> Option<f64> {
        self.events.first().map(|e| e.at)
    }

    fn insert(&mut self, event: Event<T>) {
        let index = self.events.partition_point(|other| {
            other.at < event.at || (other.at == event.at && other.sequence < event.sequence)
        });
        self.events.insert(index, event);
    }

    pub fn schedule(&mut self, payload: T, ms: f64, repeat: bool) -> Result<u64, &'static str> {
        if self.events.len() >= self.capacity {
            return Err("Timer capacity exceeded");
        }
        let duration = if repeat {
            delay(ms).max(1.0)
        } else {
            delay(ms)
        };
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Timer sequence exhausted")?;
        self.sequence = sequence;
        self.insert(Event {
            id: sequence,
            sequence,
            at: self.now + duration,
            payload,
            interval: if repeat { duration } else { 0.0 },
        });
        Ok(sequence)
    }

    pub fn cancel(&mut self, id: u64) {
        self.events.retain(|e| e.id != id);
    }

    pub fn pop(&mut self) -> Result<Option<Event<T>>, &'static str> {
        let Some(first) = self.events.first() else {
            return Ok(None);
        };
        let sequence = if first.interval > 0.0 {
            self.sequence
                .checked_add(1)
                .ok_or("Timer sequence exhausted")?
        } else {
            self.sequence
        };
        let event = self.events.remove(0);
        self.now = self.now.max(event.at);
        if event.interval > 0.0 {
            self.sequence = sequence;
            let mut next = event.clone();
            next.at = self.now + next.interval;
            next.sequence = sequence;
            self.insert(next);
        }
        Ok(Some(event))
    }
}
