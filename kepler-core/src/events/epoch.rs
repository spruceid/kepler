use crate::events::{Delegation, Event, Invocation, Revocation};

#[derive(Debug)]
pub struct Epoch {
    parents: Vec<[u8; 32]>,
    seq: u64,
    events: Vec<Event>,
}

impl Epoch {
    pub(crate) fn new(seq: u64, parents: Vec<[u8; 32]>) -> Self {
        Self {
            seq,
            parents,
            events: Vec::new(),
        }
    }

    pub(crate) fn into_inner(self) -> (u64, Vec<[u8; 32]>, Vec<Event>) {
        (self.seq, self.parents, self.events)
    }

    pub fn add_event(&mut self, event: Event) -> &mut Self {
        self.events.push(event);
        self
    }

    pub fn add_delegation(&mut self, delegation: Delegation) -> &mut Self {
        self.add_event(Event::Delegation(delegation))
    }

    pub fn add_invocation(&mut self, invocation: Invocation) -> &mut Self {
        self.add_event(Event::Invocation(invocation))
    }

    pub fn add_revocation(&mut self, revocation: Revocation) -> &mut Self {
        self.add_event(Event::Revocation(revocation))
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn parents(&self) -> &[[u8; 32]] {
        &self.parents
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
