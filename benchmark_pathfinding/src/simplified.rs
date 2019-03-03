use abstutil::Timer;
use geom::Distance;
use map_model::{DirectedRoadID, LaneID, LaneType, Map, Path, PathRequest, PathStep};
use petgraph::graph::{Graph, NodeIndex};
use std::collections::{HashMap, VecDeque};

// TODO Make the graph smaller by considering RoadID, or even (directed?) bundles of roads based on
// OSM way.
pub struct MapPathfinder {
    graph: Graph<DirectedRoadID, Distance>,
    nodes: HashMap<DirectedRoadID, NodeIndex<u32>>,
}

impl MapPathfinder {
    pub fn new(map: &Map, lane_types: Vec<LaneType>) -> MapPathfinder {
        let mut g = MapPathfinder {
            graph: Graph::new(),
            nodes: HashMap::new(),
        };

        for r in map.all_roads() {
            // TODO Technically, only if there are any matching lane types
            if !r.children_forwards.is_empty() {
                let id = r.id.forwards();
                g.nodes.insert(id, g.graph.add_node(id));
            }
            if !r.children_backwards.is_empty() {
                let id = r.id.backwards();
                g.nodes.insert(id, g.graph.add_node(id));
            }
        }

        for t in map.all_turns().values() {
            let src_l = map.get_l(t.id.src);
            let dst_l = map.get_l(t.id.dst);
            if lane_types.contains(&src_l.lane_type) && lane_types.contains(&dst_l.lane_type) {
                let src = g.get_node(t.id.src, map);
                let dst = g.get_node(t.id.dst, map);
                // First length arbitrarily wins.
                if !g.graph.contains_edge(src, dst) {
                    g.graph.add_edge(src, dst, src_l.length() + t.geom.length());
                }
            }
        }

        println!(
            "{} nodes, {} edges",
            g.graph.node_count(),
            g.graph.edge_count()
        );

        g
    }

    fn get_node(&self, lane: LaneID, map: &Map) -> NodeIndex<u32> {
        self.nodes[&map.get_l(lane).get_directed_parent(map)]
    }

    pub fn pathfind(&self, req: &PathRequest, map: &Map, timer: &mut Timer) -> Option<Path> {
        let start_node = self.get_node(req.start.lane(), map);
        let end_node = self.get_node(req.end.lane(), map);
        let end_pt = map.get_l(req.end.lane()).first_pt();

        let (_, raw_nodes) = petgraph::algo::astar(
            &self.graph,
            start_node,
            |n| n == end_node,
            |e| *e.weight(),
            |n| {
                let dr = self.graph[n];
                let r = map.get_r(dr.id);
                if dr.forwards {
                    end_pt.dist_to(r.center_pts.last_pt())
                } else {
                    end_pt.dist_to(r.center_pts.first_pt())
                }
            },
        )?;

        // TODO implement the more complicated logic
        if map.get_l(req.start.lane()).is_sidewalk() {
            return None;
        }

        // TODO windows(2) would be fine for peeking, except it drops the last element for odd
        // cardinality
        let mut nodes = VecDeque::from(raw_nodes);

        let mut steps: Vec<PathStep> = Vec::new();
        while !nodes.is_empty() {
            let n = nodes.pop_front().unwrap();
            let dr = self.graph[n];
            if steps.is_empty() {
                steps.push(PathStep::Lane(req.start.lane()));
            } else {
                let from_lane = match steps.last() {
                    Some(PathStep::Lane(l)) => *l,
                    _ => unreachable!(),
                };
                if let Some(turn) = map.get_turns_from_lane(from_lane).into_iter().find(|t| {
                    let l = map.get_l(t.id.dst);
                    if l.get_directed_parent(map) == dr {
                        nodes.is_empty()
                            || map.get_turns_from_lane(l.id).into_iter().any(|t2| {
                                map.get_l(t2.id.dst).get_directed_parent(map)
                                    == self.graph[nodes[0]]
                            })
                    } else {
                        false
                    }
                }) {
                    steps.push(PathStep::Turn(turn.id));
                    steps.push(PathStep::Lane(turn.id.dst));
                } else {
                    if steps.len() == 1 {
                        timer.warn(format!(
                            "Started in the wrong lane, can't turn from {} to {}",
                            from_lane, dr
                        ));
                        return None;
                    } else {
                        timer.warn(format!(
                            "Can't find a turn from {} to {}. Path so far: {:?}",
                            from_lane, dr, steps
                        ));
                        return None;
                    }
                }
            }
        }
        Some(Path::new(map, steps, req.end.dist_along()))
    }
}