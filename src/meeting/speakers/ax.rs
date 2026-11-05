//! What a provider adapter reads from another app's accessibility tree,
//! behind a trait so the adapters run against synthetic trees in tests and
//! against macOS Accessibility in the app (`native.rs`).

use std::time::{Duration, Instant};

/// One element. Each method is one bounded request to the other app; the
/// adapters ask for text only inside a call they have already verified.
pub trait AxNode: Sized + Clone {
    fn role(&self) -> Option<String>;
    /// Chromium's `AXDOMClassList`.
    fn classes(&self) -> Vec<String>;
    /// Chromium's `AXDOMIdentifier`.
    fn dom_id(&self) -> Option<String>;
    fn title(&self) -> Option<String>;
    fn description(&self) -> Option<String>;
    fn value(&self) -> Option<String>;
    /// Every child, or `None` when the app could not give them all (a failed
    /// request, or more than [`Budget::MAX_CHILDREN`]). Adapters treat that
    /// as an incomplete reading, which never names anyone.
    fn children(&self) -> Option<Vec<Self>>;
    /// The same live element as `other`, which identifies a bound window or
    /// call region across readings.
    fn same_element(&self, other: &Self) -> bool;
}

pub fn has_class<N: AxNode>(node: &N, class: &str) -> bool {
    node.classes().iter().any(|known| known == class)
}

/// How much of the other app one reading may walk.
pub struct Budget {
    nodes_left: usize,
    deadline: Instant,
    truncated: bool,
}

impl Budget {
    pub const MAX_DEPTH: usize = 64;
    pub const MAX_CHILDREN: usize = 512;

    pub fn new(nodes: usize, time: Duration) -> Self {
        Self {
            nodes_left: nodes,
            deadline: Instant::now() + time,
            truncated: false,
        }
    }

    /// Takes one element from the budget; `false` once it is spent, and the
    /// reading is then incomplete, which never counts as evidence.
    pub fn visit(&mut self) -> bool {
        if self.truncated || self.nodes_left == 0 || Instant::now() >= self.deadline {
            self.truncated = true;
            return false;
        }
        self.nodes_left -= 1;
        true
    }

    #[cfg(test)]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// `node`'s children for a walk at `depth`, or `None` when the walk would
    /// leave something unread: a depth past the limit with children below
    /// it, a failed read, or too many children.
    pub fn children_at<N: AxNode>(
        &mut self,
        node: &N,
        depth: usize,
        limit: usize,
    ) -> Option<Vec<N>> {
        let children = node.children();
        let complete = match &children {
            Some(children) => depth < limit || children.is_empty(),
            None => false,
        };
        if !complete {
            self.truncated = true;
            return None;
        }
        children
    }
}

/// What an adapter made of the call app this time.
#[derive(Debug, Eq, PartialEq)]
pub enum Reading {
    /// Exactly one remote participant, by a valid name, marked as speaking.
    Speaking(String),
    /// The call shows participants and none other than the user is marked.
    Silent,
    /// The call is there but its marks can't be trusted: two people marked,
    /// a marked participant without a readable name, a muted one marked.
    Uncertain,
}

/// The call an adapter found, `region` being what identifies it from one
/// reading to the next: the meeting window or the huddle's container.
pub struct Call<N> {
    pub region: N,
    pub reading: Reading,
}

/// Why there was no call to read.
#[derive(Debug, Eq, PartialEq)]
pub enum NoCall {
    /// Nothing that looks like this provider's call.
    Absent,
    /// More than one candidate call, or the walk ran out of budget: unknown.
    Unknown,
}

/// Counts the call regions bound so far, so a reading from another window
/// or a relaunched app never continues an earlier stretch.
pub struct Binder<N> {
    pid: i32,
    region: Option<N>,
    epoch: u32,
}

impl<N: AxNode> Default for Binder<N> {
    fn default() -> Self {
        Self {
            pid: 0,
            region: None,
            epoch: 0,
        }
    }
}

impl<N: AxNode> Binder<N> {
    /// The binding for `region` of process `pid`: the current one when it is
    /// the same element, else a new one.
    pub fn bind(&mut self, pid: i32, region: N) -> u32 {
        let same = self.pid == pid
            && self
                .region
                .as_ref()
                .is_some_and(|bound| bound.same_element(&region));
        if !same {
            self.epoch += 1;
            self.pid = pid;
            self.region = Some(region);
        }
        self.epoch
    }

    /// The bound region, for an adapter that can read it again directly.
    pub fn region(&self, pid: i32) -> Option<&N> {
        (self.pid == pid).then_some(self.region.as_ref()).flatten()
    }

    /// The call is gone; the next one is new even if it reuses the element.
    pub fn lose(&mut self) {
        self.region = None;
    }
}

#[cfg(test)]
pub mod fake {
    //! Synthetic trees shaped like the published accessibility evidence.
    //! No fixture comes from a real call or a real person.
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::AxNode;

    #[derive(Default)]
    pub struct Data {
        pub role: String,
        pub classes: Vec<String>,
        pub dom_id: Option<String>,
        pub title: Option<String>,
        pub description: Option<String>,
        pub value: Option<String>,
        pub children: Vec<Node>,
        /// The app fails to list this element's children.
        pub unreadable: bool,
    }

    /// Shared reads count text requests, to prove where adapters look.
    #[derive(Clone)]
    pub struct Node {
        pub data: Rc<Data>,
        pub text_reads: Rc<RefCell<Vec<String>>>,
        pub alive: Rc<Cell<bool>>,
    }

    impl Node {
        pub fn element(role: &str) -> Builder {
            Builder(Data {
                role: role.into(),
                ..Data::default()
            })
        }

        fn text(&self, field: &Option<String>) -> Option<String> {
            if !self.alive.get() {
                return None;
            }
            if let Some(text) = field {
                self.text_reads.borrow_mut().push(text.clone());
            }
            field.clone()
        }

        /// Every node below, this one included.
        pub fn all(&self) -> Vec<Node> {
            let mut nodes = vec![self.clone()];
            for child in &self.data.children {
                nodes.extend(child.all());
            }
            nodes
        }

        /// Text read anywhere in this tree.
        pub fn reads(&self) -> Vec<String> {
            self.text_reads.borrow().clone()
        }
    }

    /// `inner` wrapped in `levels` plain groups.
    pub fn nested(levels: usize, inner: Builder) -> Builder {
        (0..levels).fold(inner, |inner, _| Node::element("AXGroup").child(inner))
    }

    /// `count` empty groups and then `last`, as siblings.
    pub fn crowd(count: usize, last: Builder) -> Vec<Builder> {
        let mut crowd = (0..count)
            .map(|_| Node::element("AXGroup"))
            .collect::<Vec<_>>();
        crowd.push(last);
        crowd
    }

    pub struct Builder(Data);

    impl Builder {
        pub fn class(mut self, class: &str) -> Self {
            self.0.classes.push(class.into());
            self
        }
        pub fn id(mut self, id: &str) -> Self {
            self.0.dom_id = Some(id.into());
            self
        }
        pub fn title(mut self, title: &str) -> Self {
            self.0.title = Some(title.into());
            self
        }
        pub fn description(mut self, description: &str) -> Self {
            self.0.description = Some(description.into());
            self
        }
        pub fn value(mut self, value: &str) -> Self {
            self.0.value = Some(value.into());
            self
        }
        /// The app can't list this element's children.
        pub fn unreadable(mut self) -> Self {
            self.0.unreadable = true;
            self
        }
        pub fn child(mut self, child: Builder) -> Self {
            self.0.children.push(child.build_detached());
            self
        }
        pub fn children(mut self, children: impl IntoIterator<Item = Builder>) -> Self {
            for child in children {
                self = self.child(child);
            }
            self
        }
        fn build_detached(self) -> Node {
            Node {
                data: Rc::new(self.0),
                text_reads: Rc::default(),
                alive: Rc::new(Cell::new(true)),
            }
        }
        /// The finished tree, every node sharing one read log.
        pub fn build(self) -> Node {
            let log = Rc::new(RefCell::new(Vec::new()));
            let alive = Rc::new(Cell::new(true));
            share(self.build_detached(), &log, &alive)
        }
    }

    fn share(node: Node, log: &Rc<RefCell<Vec<String>>>, alive: &Rc<Cell<bool>>) -> Node {
        let data = Rc::try_unwrap(node.data).unwrap_or_else(|_| unreachable!());
        let children = data
            .children
            .into_iter()
            .map(|child| share(child, log, alive))
            .collect();
        Node {
            data: Rc::new(super::fake::Data { children, ..data }),
            text_reads: log.clone(),
            alive: alive.clone(),
        }
    }

    impl AxNode for Node {
        fn role(&self) -> Option<String> {
            self.alive.get().then(|| self.data.role.clone())
        }
        fn classes(&self) -> Vec<String> {
            if self.alive.get() {
                self.data.classes.clone()
            } else {
                Vec::new()
            }
        }
        fn dom_id(&self) -> Option<String> {
            self.alive.get().then(|| self.data.dom_id.clone()).flatten()
        }
        fn title(&self) -> Option<String> {
            self.text(&self.data.title)
        }
        fn description(&self) -> Option<String> {
            self.text(&self.data.description)
        }
        fn value(&self) -> Option<String> {
            self.text(&self.data.value)
        }
        fn children(&self) -> Option<Vec<Self>> {
            if !self.alive.get() {
                Some(Vec::new())
            } else if self.data.unreadable || self.data.children.len() > super::Budget::MAX_CHILDREN
            {
                None
            } else {
                Some(self.data.children.clone())
            }
        }
        fn same_element(&self, other: &Self) -> bool {
            std::rc::Rc::ptr_eq(&self.data, &other.data)
        }
    }
}
