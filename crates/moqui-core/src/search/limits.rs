//! Request-scoped search budgets; nested backend calls share the same deadline.
use moqui_lib::error::Error;
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
pub struct QueryLimits {
    pub timeout: Duration,
    pub buckets: usize,
}
impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            buckets: 10_000,
        }
    }
}
#[derive(Clone, Copy)]
struct Active {
    deadline: Instant,
    buckets: usize,
}
thread_local! {static ACTIVE:RefCell<Option<Active>>=const{RefCell::new(None)};}
struct Restore(Option<Active>);
impl Drop for Restore {
    fn drop(&mut self) {
        ACTIVE.with(|active| *active.borrow_mut() = self.0);
    }
}
pub fn with_limits<T>(limits: QueryLimits, body: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    let next = Active {
        deadline: Instant::now() + limits.timeout,
        buckets: limits.buckets.clamp(1, 65_536),
    };
    let old = ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let old = *active;
        *active = Some(match old {
            Some(old) => Active {
                deadline: old.deadline.min(next.deadline),
                buckets: old.buckets.min(next.buckets),
            },
            None => next,
        });
        old
    });
    let _restore = Restore(old);
    body()
}
pub(crate) fn remaining() -> Result<Option<Duration>, Error> {
    ACTIVE.with(|active| match *active.borrow() {
        None => Ok(None),
        Some(active) => {
            let remaining = active.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                Err(Error::new("report query timeout exceeded"))
            } else {
                Ok(Some(remaining))
            }
        }
    })
}
pub(crate) fn buckets() -> usize {
    ACTIVE.with(|active| active.borrow().map_or(65_536, |a| a.buckets))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_limits_restore_without_leaking() {
        assert_eq!(buckets(), 65_536);
        with_limits(
            QueryLimits {
                buckets: 5,
                ..Default::default()
            },
            || {
                assert_eq!(buckets(), 5);
                with_limits(
                    QueryLimits {
                        buckets: 2,
                        ..Default::default()
                    },
                    || {
                        assert_eq!(buckets(), 2);
                        Ok(())
                    },
                )?;
                assert_eq!(buckets(), 5);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(buckets(), 65_536);
    }
}
