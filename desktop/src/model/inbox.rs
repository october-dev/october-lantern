//! Waiting and the agent list (Swift `AppModel` derived state): which turns are new, which are
//! earlier, which you dismissed, and which deserve a notification. Turns are keyed `agentId@stateSince`,
//! so dismissing or seeing one never hides the agent's next turn.
use super::{Agent, AgentId, AgentState, Prefs};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Waiting, newest first, split the way the panel shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inbox {
    /// Turns you haven't looked at yet; a question for you always counts. The badge is its length.
    pub new: Vec<AgentId>,
    /// Your-turn waits you've already seen.
    pub earlier: Vec<AgentId>,
}

/// Persisted bookkeeping behind `Inbox`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Turns {
    pub seen: HashSet<String>,
    pub dismissed: HashSet<String>,
    #[serde(skip)]
    known: HashSet<String>,
    #[serde(skip)]
    observed_once: bool,
}

/// Needs you, then your turn, then working, then the rest; not-live app sessions last; newest first.
pub fn rank(agents: &mut [Agent]) {
    let order = |agent: &Agent| if agent.is_live() { agent.state.rank() } else { 4 };
    agents.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| b.state_since.unwrap_or(0.0).total_cmp(&a.state_since.unwrap_or(0.0))));
}

fn waiting<'a>(agents: &'a [Agent], prefs: &'a Prefs) -> impl Iterator<Item = &'a Agent> {
    agents.iter().filter(|agent| agent.is_live() && agent.state.wants_you() && prefs.counts(&agent.kind))
}

impl Turns {
    /// Live waits in harnesses you haven't muted, not dismissed, newest first. An agent with a
    /// card waiting for you (`carded`) is always new, whatever its own state, and isn't dismissed
    /// (only answered).
    pub fn inbox(&self, agents: &[Agent], prefs: &Prefs, carded: &HashSet<String>) -> Inbox {
        let mut list: Vec<&Agent> = agents
            .iter()
            .filter(|agent| agent.is_live() && prefs.counts(&agent.kind))
            .filter(|agent| carded.contains(&agent.id) || (agent.state.wants_you() && !self.dismissed.contains(&agent.turn_key())))
            .collect();
        list.sort_by(|a, b| b.state_since.unwrap_or(0.0).total_cmp(&a.state_since.unwrap_or(0.0)));
        let (new, earlier): (Vec<&Agent>, Vec<&Agent>) = list.into_iter().partition(|agent| {
            agent.state == AgentState::NeedsInput || carded.contains(&agent.id) || !self.seen.contains(&agent.turn_key())
        });
        Inbox { new: new.into_iter().map(|a| a.id.clone()).collect(), earlier: earlier.into_iter().map(|a| a.id.clone()).collect() }
    }

    /// "Done": hides these turns from Waiting until the agent's state changes again. The keys are
    /// the turns the person saw; a key for a turn that has since moved on matches nothing.
    pub fn dismiss(&mut self, turn_keys: &[String], agents: &[Agent]) {
        self.dismissed.extend(turn_keys.iter().cloned());
        let live: HashSet<String> = agents.iter().map(Agent::turn_key).collect();
        self.dismissed.retain(|key| live.contains(key));
    }

    /// Marks everything in Waiting as seen (leaving Waiting). Returns whether anything changed.
    pub fn mark_seen(&mut self, agents: &[Agent], prefs: &Prefs) -> bool {
        let inbox = self.inbox(agents, prefs, &HashSet::new());
        let keys: Vec<String> =
            inbox.new.iter().chain(&inbox.earlier).filter_map(|id| agents.iter().find(|a| &a.id == id)).map(Agent::turn_key).collect();
        if keys.iter().all(|key| self.seen.contains(key)) {
            return false;
        }
        let live: HashSet<String> = agents.iter().map(Agent::turn_key).collect();
        self.seen.extend(keys);
        self.seen.retain(|key| live.contains(key));
        true
    }

    /// A new snapshot: the turns that just started waiting, for notifications. On the first snapshot
    /// after launch nothing notifies, and your-turn waits older than two hours count as seen (never
    /// input requests), so opening Lantern isn't a wall of old sessions.
    pub fn observe(&mut self, agents: &[Agent], prefs: &Prefs, now_ms: f64) -> Vec<AgentId> {
        let current: Vec<&Agent> = waiting(agents, prefs).collect();
        let keys: HashSet<String> = current.iter().map(|agent| agent.turn_key()).collect();
        if !self.observed_once {
            self.observed_once = true;
            let stale = now_ms - 2.0 * 3600.0 * 1000.0;
            for agent in &current {
                if agent.state != AgentState::NeedsInput && agent.state_since.unwrap_or(f64::MIN) < stale {
                    self.seen.insert(agent.turn_key());
                }
            }
            self.known = keys;
            return Vec::new();
        }
        let fresh = current
            .iter()
            .filter(|agent| {
                let key = agent.turn_key();
                !self.known.contains(&key) && !self.seen.contains(&key)
            })
            .map(|agent| agent.id.clone())
            .collect();
        self.known = keys;
        fresh
    }

    /// The engine restarted: the next snapshot is a first snapshot again.
    pub fn restart(&mut self) {
        self.observed_once = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentKind;

    fn agent(id: &str, state: AgentState, since: f64) -> Agent {
        Agent { id: id.into(), kind: AgentKind::new("claude"), state, state_since: Some(since), ..Default::default() }
    }
    const NOW: f64 = 1_790_000_000_000.0;
    const HOUR: f64 = 3_600_000.0;

    #[test]
    fn questions_stay_new_even_when_seen() {
        let prefs = Prefs::default();
        let agents = vec![agent("a", AgentState::NeedsInput, NOW), agent("b", AgentState::Waiting, NOW - 1.0)];
        let mut turns = Turns::default();
        turns.observe(&agents, &prefs, NOW);
        assert!(turns.mark_seen(&agents, &prefs));
        let inbox = turns.inbox(&agents, &prefs, &HashSet::new());
        assert_eq!(inbox.new, vec!["a"]);
        assert_eq!(inbox.earlier, vec!["b"]);
    }

    #[test]
    fn a_waiting_card_is_new_whatever_the_agent_says_and_isnt_dismissed() {
        let prefs = Prefs::default();
        // Working, or a turn you dismissed: a card waiting for you still puts it first.
        let agents = vec![agent("a", AgentState::Working, NOW), agent("b", AgentState::Waiting, NOW - 1.0)];
        let mut turns = Turns::default();
        turns.dismiss(&[agents[1].turn_key()], &agents);
        assert!(turns.inbox(&agents, &prefs, &HashSet::new()).new.is_empty());
        let carded: HashSet<String> = ["a".to_string(), "b".to_string()].into();
        turns.mark_seen(&agents, &prefs);
        let inbox = turns.inbox(&agents, &prefs, &carded);
        assert_eq!(inbox.new, vec!["a", "b"]);
        assert!(inbox.earlier.is_empty());
    }

    #[test]
    fn the_engines_cards_decode() {
        let launches: Vec<crate::model::CardLaunch> = serde_json::from_str(
            r#"[{"id":"l","kind":"october","cwd":"/tmp","title":"Book","agentId":"october:1:2","tools":true,"createdAt":1,
            "endedAt":null,"status":"Working","steps":[{"id":"a","title":"Search","done":true}],"revision":2,
            "cards":[{"id":"c","kind":"ask","state":"waiting","revision":1,"createdAt":1,"resolvedAt":null,"answer":null,
            "question":"Which?","details":null,"options":["A","B"]}]}]"#,
        )
        .unwrap();
        assert_eq!(launches[0].waiting().map(|c| c.options.clone()).next(), Some(Some(vec!["A".to_string(), "B".to_string()])));
        assert!(launches[0].steps[0].done);
    }

    #[test]
    fn dismissing_a_turn_never_hides_the_next_one() {
        let prefs = Prefs::default();
        let first = vec![agent("a", AgentState::Waiting, NOW)];
        let mut turns = Turns::default();
        let shown = first[0].turn_key();
        turns.dismiss(std::slice::from_ref(&shown), &first);
        assert!(turns.inbox(&first, &prefs, &HashSet::new()).new.is_empty());
        let next = vec![agent("a", AgentState::Waiting, NOW + 5.0)];
        assert_eq!(turns.inbox(&next, &prefs, &HashSet::new()).new, vec!["a"]);
        // A delayed Dismiss carrying the old key doesn't hide the new turn either.
        turns.dismiss(&[shown], &next);
        assert_eq!(turns.inbox(&next, &prefs, &HashSet::new()).new, vec!["a"]);
    }

    #[test]
    fn muted_harnesses_and_finished_app_sessions_stay_out() {
        let prefs = Prefs { muted: vec!["claude".into()], ..Default::default() };
        let mut codex = agent("c", AgentState::Waiting, NOW);
        codex.kind = AgentKind::new("codex");
        let mut gone = agent("g", AgentState::Waiting, NOW);
        gone.kind = AgentKind::new("codex");
        gone.live = Some(false);
        let agents = vec![agent("a", AgentState::Waiting, NOW), codex, gone];
        assert_eq!(Turns::default().inbox(&agents, &prefs, &HashSet::new()).new, vec!["c"]);
    }

    #[test]
    fn first_snapshot_is_quiet_and_seens_old_waits_but_not_questions() {
        let prefs = Prefs::default();
        let agents = vec![agent("old", AgentState::Waiting, NOW - 3.0 * HOUR), agent("ask", AgentState::NeedsInput, NOW - 3.0 * HOUR)];
        let mut turns = Turns::default();
        assert!(turns.observe(&agents, &prefs, NOW).is_empty());
        let inbox = turns.inbox(&agents, &prefs, &HashSet::new());
        assert_eq!(inbox.new, vec!["ask"]);
        assert_eq!(inbox.earlier, vec!["old"]);
    }

    #[test]
    fn later_snapshots_notify_each_new_turn_once() {
        let prefs = Prefs::default();
        let mut turns = Turns::default();
        turns.observe(&[agent("a", AgentState::Working, NOW)], &prefs, NOW);
        assert_eq!(turns.observe(&[agent("a", AgentState::Waiting, NOW + 1.0)], &prefs, NOW), vec!["a"]);
        assert!(turns.observe(&[agent("a", AgentState::Waiting, NOW + 1.0)], &prefs, NOW).is_empty());
        assert_eq!(turns.observe(&[agent("a", AgentState::NeedsInput, NOW + 2.0)], &prefs, NOW), vec!["a"]);
    }

    #[test]
    fn rank_orders_by_state_then_newest_with_finished_sessions_last() {
        let mut gone = agent("gone", AgentState::NeedsInput, NOW + 9.0);
        gone.live = Some(false);
        let mut agents = vec![
            agent("idle", AgentState::Idle, NOW),
            gone,
            agent("work", AgentState::Working, NOW),
            agent("turn-old", AgentState::Waiting, NOW),
            agent("turn-new", AgentState::Waiting, NOW + 1.0),
            agent("ask", AgentState::NeedsInput, NOW),
        ];
        rank(&mut agents);
        let order: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(order, ["ask", "turn-new", "turn-old", "work", "idle", "gone"]);
    }
}
