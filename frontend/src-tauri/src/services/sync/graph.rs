use grafyn_sync_protocol::{
    OperationId, VerifiedOperation, MAX_CAUSAL_PARENTS, MAX_ENVELOPE_JSON_BYTES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub(crate) const MAX_CAUSAL_GRAPH_OPERATIONS: usize = 4096;
pub(crate) const MAX_CAUSAL_GRAPH_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphLimits {
    max_operations: usize,
    max_envelope_bytes: usize,
}

impl Default for GraphLimits {
    fn default() -> Self {
        Self {
            max_operations: MAX_CAUSAL_GRAPH_OPERATIONS,
            max_envelope_bytes: MAX_CAUSAL_GRAPH_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GraphError {
    InvalidParents,
    InvalidEnvelopeBytes,
    IdentityCollision(OperationId),
    OperationLimitExceeded,
    ByteLimitExceeded,
    UnknownParent(OperationId),
    CausalCycle,
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParents => {
                formatter.write_str("causal parents must be bounded, sorted, and unique")
            }
            Self::InvalidEnvelopeBytes => formatter.write_str("invalid canonical envelope size"),
            Self::IdentityCollision(id) => write!(formatter, "operation ID collision: {id}"),
            Self::OperationLimitExceeded => {
                formatter.write_str("causal graph operation limit exceeded")
            }
            Self::ByteLimitExceeded => formatter.write_str("causal graph byte limit exceeded"),
            Self::UnknownParent(id) => write!(formatter, "unknown causal parent: {id}"),
            Self::CausalCycle => formatter.write_str("causal operation cycle detected"),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphInsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CausalNode {
    operation_id: OperationId,
    causal_parents: Vec<OperationId>,
    envelope_bytes: usize,
}

impl CausalNode {
    pub(crate) fn new(
        operation_id: OperationId,
        causal_parents: Vec<OperationId>,
        envelope_bytes: usize,
    ) -> Result<Self, GraphError> {
        if envelope_bytes == 0 || envelope_bytes > MAX_ENVELOPE_JSON_BYTES {
            return Err(GraphError::InvalidEnvelopeBytes);
        }
        if causal_parents.len() > MAX_CAUSAL_PARENTS
            || causal_parents.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(GraphError::InvalidParents);
        }
        if causal_parents.binary_search(&operation_id).is_ok() {
            return Err(GraphError::CausalCycle);
        }
        Ok(Self {
            operation_id,
            causal_parents,
            envelope_bytes,
        })
    }

    pub(crate) fn from_verified(
        operation: &VerifiedOperation,
        envelope_bytes: usize,
    ) -> Result<Self, GraphError> {
        Self::new(
            *operation.operation_id(),
            operation.operation().causal_parents().to_vec(),
            envelope_bytes,
        )
    }

    pub(crate) const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub(crate) fn causal_parents(&self) -> &[OperationId] {
        &self.causal_parents
    }

    pub(crate) const fn envelope_bytes(&self) -> usize {
        self.envelope_bytes
    }
}

#[derive(Debug)]
pub(crate) struct CausalGraph {
    nodes: BTreeMap<OperationId, CausalNode>,
    total_envelope_bytes: usize,
    limits: GraphLimits,
}

impl Default for CausalGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CausalGraph {
    pub(crate) fn new() -> Self {
        Self::with_limits(GraphLimits::default())
    }

    fn with_limits(limits: GraphLimits) -> Self {
        Self {
            nodes: BTreeMap::new(),
            total_envelope_bytes: 0,
            limits,
        }
    }

    pub(crate) fn insert_verified(
        &mut self,
        operation: &VerifiedOperation,
        envelope_bytes: usize,
    ) -> Result<GraphInsertOutcome, GraphError> {
        self.insert(CausalNode::from_verified(operation, envelope_bytes)?)
    }

    pub(crate) fn insert(
        &mut self,
        candidate: CausalNode,
    ) -> Result<GraphInsertOutcome, GraphError> {
        if let Some(existing) = self.nodes.get(candidate.operation_id()) {
            return if existing == &candidate {
                Ok(GraphInsertOutcome::Duplicate)
            } else {
                Err(GraphError::IdentityCollision(*candidate.operation_id()))
            };
        }
        if self.nodes.len() >= self.limits.max_operations {
            return Err(GraphError::OperationLimitExceeded);
        }
        let total = self
            .total_envelope_bytes
            .checked_add(candidate.envelope_bytes())
            .ok_or(GraphError::ByteLimitExceeded)?;
        if total > self.limits.max_envelope_bytes {
            return Err(GraphError::ByteLimitExceeded);
        }

        let id = *candidate.operation_id();
        self.nodes.insert(id, candidate);
        if graph_has_cycle(&self.nodes) {
            self.nodes.remove(&id);
            return Err(GraphError::CausalCycle);
        }
        self.total_envelope_bytes = total;
        Ok(GraphInsertOutcome::Inserted)
    }

    pub(crate) fn validate_complete(
        &self,
        already_applied: &BTreeSet<OperationId>,
    ) -> Result<Vec<OperationId>, GraphError> {
        let known: BTreeSet<_> = self
            .nodes
            .keys()
            .copied()
            .chain(already_applied.iter().copied())
            .collect();
        let unknown = self
            .nodes
            .values()
            .flat_map(CausalNode::causal_parents)
            .filter(|parent| !known.contains(parent))
            .copied()
            .min();
        if let Some(parent) = unknown {
            return Err(GraphError::UnknownParent(parent));
        }
        if graph_has_cycle(&self.nodes) {
            return Err(GraphError::CausalCycle);
        }
        let (ordered, remaining) = ready_order(&self.nodes, already_applied);
        if !remaining.is_empty() {
            return Err(GraphError::CausalCycle);
        }
        Ok(ordered)
    }

    pub(crate) fn drain_ready(
        &mut self,
        already_applied: &BTreeSet<OperationId>,
    ) -> Result<Vec<OperationId>, GraphError> {
        if graph_has_cycle(&self.nodes) {
            return Err(GraphError::CausalCycle);
        }
        let (ordered, remaining) = ready_order(&self.nodes, already_applied);
        let retained: BTreeSet<_> = remaining.into_iter().collect();
        self.nodes.retain(|id, _| retained.contains(id));
        self.total_envelope_bytes = self.nodes.values().map(CausalNode::envelope_bytes).sum();
        Ok(ordered)
    }

    pub(crate) fn get(&self, id: &OperationId) -> Option<&CausalNode> {
        self.nodes.get(id)
    }

    pub(crate) fn ids(&self) -> Vec<OperationId> {
        self.nodes.keys().copied().collect()
    }

    pub(crate) fn deferred_ids(&self) -> Vec<OperationId> {
        self.ids()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

fn ready_order(
    nodes: &BTreeMap<OperationId, CausalNode>,
    already_applied: &BTreeSet<OperationId>,
) -> (Vec<OperationId>, Vec<OperationId>) {
    let mut remaining: BTreeSet<_> = nodes.keys().copied().collect();
    let mut satisfied = already_applied.clone();
    let mut ordered = Vec::with_capacity(nodes.len());
    loop {
        let ready = remaining.iter().find(|id| {
            already_applied.contains(id)
                || nodes
                    .get(id)
                    .expect("remaining node exists")
                    .causal_parents()
                    .iter()
                    .all(|parent| satisfied.contains(parent))
        });
        let Some(id) = ready.copied() else {
            break;
        };
        remaining.remove(&id);
        satisfied.insert(id);
        if !already_applied.contains(&id) {
            ordered.push(id);
        }
    }
    (ordered, remaining.into_iter().collect())
}

fn graph_has_cycle(nodes: &BTreeMap<OperationId, CausalNode>) -> bool {
    let mut indegree: BTreeMap<_, usize> = nodes.keys().copied().map(|id| (id, 0)).collect();
    let mut children: BTreeMap<OperationId, BTreeSet<OperationId>> = BTreeMap::new();
    for node in nodes.values() {
        for parent in node.causal_parents() {
            if nodes.contains_key(parent) {
                *indegree
                    .get_mut(node.operation_id())
                    .expect("graph node exists") += 1;
                children
                    .entry(*parent)
                    .or_default()
                    .insert(*node.operation_id());
            }
        }
    }
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        if let Some(next) = children.get(&id) {
            for child in next {
                let degree = indegree.get_mut(child).expect("child node exists");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(*child);
                }
            }
        }
    }
    visited != nodes.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grafyn_sync_protocol::OperationId;
    use std::collections::BTreeSet;

    fn id(byte: u8) -> OperationId {
        OperationId::from_bytes([byte; 32])
    }

    fn node(byte: u8, parents: Vec<OperationId>, envelope_bytes: usize) -> CausalNode {
        CausalNode::new(id(byte), parents, envelope_bytes).unwrap()
    }

    #[test]
    fn causal_parents_must_be_strictly_sorted_and_unique() {
        assert_eq!(
            CausalNode::new(id(3), vec![id(2), id(1)], 10).unwrap_err(),
            GraphError::InvalidParents
        );
        assert_eq!(
            CausalNode::new(id(3), vec![id(1), id(1)], 10).unwrap_err(),
            GraphError::InvalidParents
        );
    }

    #[test]
    fn child_before_parent_is_deferred_then_drained_parent_first() {
        let mut graph = CausalGraph::new();
        assert_eq!(
            graph.insert(node(2, vec![id(1)], 20)).unwrap(),
            GraphInsertOutcome::Inserted
        );

        assert!(graph.drain_ready(&BTreeSet::new()).unwrap().is_empty());
        assert_eq!(graph.deferred_ids(), vec![id(2)]);

        graph.insert(node(1, vec![], 10)).unwrap();
        assert_eq!(
            graph.drain_ready(&BTreeSet::new()).unwrap(),
            vec![id(1), id(2)]
        );
        assert!(graph.is_empty());
    }

    #[test]
    fn reordering_and_exact_duplicates_have_one_deterministic_drain() {
        let operations = [
            node(4, vec![id(2), id(3)], 40),
            node(2, vec![id(1)], 20),
            node(3, vec![id(1)], 30),
            node(1, vec![], 10),
        ];
        let mut forward = CausalGraph::new();
        let mut reverse = CausalGraph::new();
        for operation in &operations {
            forward.insert(operation.clone()).unwrap();
        }
        for operation in operations.iter().rev() {
            reverse.insert(operation.clone()).unwrap();
        }
        assert_eq!(
            reverse.insert(operations[1].clone()).unwrap(),
            GraphInsertOutcome::Duplicate
        );

        let expected = vec![id(1), id(2), id(3), id(4)];
        assert_eq!(forward.drain_ready(&BTreeSet::new()).unwrap(), expected);
        assert_eq!(
            reverse.drain_ready(&BTreeSet::new()).unwrap(),
            vec![id(1), id(2), id(3), id(4)]
        );
    }

    #[test]
    fn identity_collision_is_rejected_without_replacing_the_first_node() {
        let mut graph = CausalGraph::new();
        let first = node(2, vec![id(1)], 20);
        graph.insert(first.clone()).unwrap();

        assert_eq!(
            graph.insert(node(2, vec![], 20)).unwrap_err(),
            GraphError::IdentityCollision(id(2))
        );
        assert_eq!(graph.get(&id(2)), Some(&first));
    }

    #[test]
    fn a_cycle_is_rejected_before_it_can_enter_the_graph() {
        let mut graph = CausalGraph::new();
        graph.insert(node(1, vec![id(2)], 10)).unwrap();

        assert_eq!(
            graph.insert(node(2, vec![id(1)], 10)).unwrap_err(),
            GraphError::CausalCycle
        );
        assert_eq!(graph.ids(), vec![id(1)]);
        assert_eq!(
            CausalNode::new(id(3), vec![id(3)], 10).unwrap_err(),
            GraphError::CausalCycle
        );
    }

    #[test]
    fn complete_validation_fails_closed_on_the_first_unknown_parent() {
        let mut graph = CausalGraph::new();
        graph.insert(node(4, vec![id(2), id(3)], 10)).unwrap();

        assert_eq!(
            graph.validate_complete(&BTreeSet::new()).unwrap_err(),
            GraphError::UnknownParent(id(2))
        );
        assert_eq!(graph.ids(), vec![id(4)]);
    }

    #[test]
    fn already_applied_parents_make_children_ready() {
        let mut graph = CausalGraph::new();
        graph.insert(node(2, vec![id(1)], 10)).unwrap();

        assert_eq!(
            graph.drain_ready(&BTreeSet::from([id(1)])).unwrap(),
            vec![id(2)]
        );
    }

    #[test]
    fn graph_count_and_aggregate_bytes_are_bounded() {
        let limits = GraphLimits {
            max_operations: 2,
            max_envelope_bytes: 25,
        };
        let mut graph = CausalGraph::with_limits(limits);
        graph.insert(node(1, vec![], 10)).unwrap();
        graph.insert(node(2, vec![], 15)).unwrap();

        assert_eq!(
            graph.insert(node(3, vec![], 1)).unwrap_err(),
            GraphError::OperationLimitExceeded
        );

        let mut byte_limited = CausalGraph::with_limits(limits);
        byte_limited.insert(node(1, vec![], 20)).unwrap();
        assert_eq!(
            byte_limited.insert(node(2, vec![], 6)).unwrap_err(),
            GraphError::ByteLimitExceeded
        );
    }

    #[test]
    fn one_envelope_cannot_exceed_the_protocol_boundary() {
        assert_eq!(
            CausalNode::new(
                id(1),
                vec![],
                grafyn_sync_protocol::MAX_ENVELOPE_JSON_BYTES + 1,
            )
            .unwrap_err(),
            GraphError::InvalidEnvelopeBytes
        );
    }
}
