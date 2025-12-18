use crate::{Event, Storage, TimeLoopError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize)]
pub struct TimelineBranch {
    pub id: String,
    pub name: String,
    pub parent_session_id: String,
    pub branch_point_event_id: String,
    #[zeroize(skip)]
    pub created_at: DateTime<Utc>,
    pub description: Option<String>,
}

pub struct BranchManager {
    storage: Storage,
}

impl BranchManager {
    pub fn new() -> crate::Result<Self> {
        let storage = Storage::new()?;
        Ok(Self { storage })
    }

    pub fn create_branch(
        &mut self,
        parent_session_id: &str,
        name: &str,
        branch_point_event_id: &str,
        description: Option<&str>,
    ) -> crate::Result<String> {
        let branch_id = Uuid::new_v4().to_string();
        let branch = TimelineBranch {
            id: branch_id.clone(),
            name: name.to_string(),
            parent_session_id: parent_session_id.to_string(),
            branch_point_event_id: branch_point_event_id.to_string(),
            created_at: Utc::now(),
            description: description.map(|s| s.to_string()),
        };

        self.storage.store_branch(&branch)?;
        Ok(branch_id)
    }

    pub fn get_branch(&self, branch_id: &str) -> crate::Result<Option<TimelineBranch>> {
        self.storage.get_branch(branch_id)
    }

    pub fn list_branches(&self) -> crate::Result<Vec<TimelineBranch>> {
        self.storage.list_branches()
    }

    pub fn get_branches_for_session(&self, session_id: &str) -> crate::Result<Vec<TimelineBranch>> {
        let all_branches = self.list_branches()?;
        let session_branches: Vec<TimelineBranch> = all_branches
            .into_iter()
            .filter(|branch| branch.parent_session_id == session_id)
            .collect();
        Ok(session_branches)
    }

    pub fn replay_branch(&self, branch_id: &str) -> crate::Result<Vec<Event>> {
        let branch = self
            .get_branch(branch_id)?
            .ok_or_else(|| TimeLoopError::Branch(format!("Branch {} not found", branch_id)))?;

        // Get all events from the parent session up to the branch point
        let parent_events = self
            .storage
            .get_events_for_session(&branch.parent_session_id)?;

        // Find the branch point event
        let branch_point_index = parent_events
            .iter()
            .position(|event| event.id == branch.branch_point_event_id)
            .ok_or_else(|| TimeLoopError::Branch("Branch point event not found".to_string()))?;

        // Return events up to and including the branch point
        Ok(parent_events[..=branch_point_index].to_vec())
    }

    pub fn get_branch_timeline(&self, branch_id: &str) -> crate::Result<BranchTimeline> {
        let branch = self
            .get_branch(branch_id)?
            .ok_or_else(|| TimeLoopError::Branch(format!("Branch {} not found", branch_id)))?;

        let parent_events = self
            .storage
            .get_events_for_session(&branch.parent_session_id)?;
        let branch_events = self.storage.get_events_for_session(branch_id)?;

        // Find the branch point
        let branch_point_index = parent_events
            .iter()
            .position(|event| event.id == branch.branch_point_event_id)
            .ok_or_else(|| TimeLoopError::Branch("Branch point event not found".to_string()))?;

        Ok(BranchTimeline {
            branch,
            parent_events: parent_events[..=branch_point_index].to_vec(),
            branch_events,
        })
    }

    pub fn merge_branch(&mut self, branch_id: &str, target_session_id: &str) -> crate::Result<()> {
        let branch_timeline = self.get_branch_timeline(branch_id)?;

        // Copy branch events to the target session
        for event in &branch_timeline.branch_events {
            let mut new_event = event.clone();
            new_event.session_id = target_session_id.to_string();
            new_event.id = Uuid::new_v4().to_string();
            self.storage.store_event(&new_event)?;
        }

        Ok(())
    }

    pub fn delete_branch(&mut self, branch_id: &str) -> crate::Result<()> {
        self.storage.delete_branch(branch_id)
    }
}

#[derive(Debug, Clone)]
pub struct BranchTimeline {
    pub branch: TimelineBranch,
    pub parent_events: Vec<Event>,
    pub branch_events: Vec<Event>,
}

impl BranchTimeline {
    pub fn get_all_events(&self) -> Vec<Event> {
        let mut all_events = self.parent_events.clone();
        all_events.extend(self.branch_events.clone());
        all_events.sort_by_key(|e| e.sequence_number);
        all_events
    }

    pub fn get_divergence_point(&self) -> Option<&Event> {
        self.parent_events.last()
    }

    pub fn get_branch_duration(&self) -> Option<chrono::Duration> {
        if let (Some(first), Some(last)) = (self.branch_events.first(), self.branch_events.last()) {
            Some(last.timestamp - first.timestamp)
        } else {
            None
        }
    }
}
