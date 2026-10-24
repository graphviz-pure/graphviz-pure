//! Recursive-descent DOT parser matching Graphviz `lib/cgraph/grammar.y`,
//! with full lexical scoping for `graph`, `node`, and `edge` defaults across
//! nested subgraphs and clusters.

use crate::lexer::{Lexer, ScanInput, ScanState, Token, TokenKind};
type AttrMap2 = std::collections::BTreeMap<String, graphviz_pure_core::AttrSym>;

use graphviz_pure_core::{
    AttrMap, BoundingBox, EdgeId, Graph, GraphKind, GraphvizError, HTML_VALUE_PREFIX, NodeId, Port,
    Result, SubgraphData, SubgraphId, apply_attrs, attr_is_html, set_attr,
};

/// bison's `YYMAXDEPTH` (grammar.c): the most states its parser stack holds.
const YYMAXDEPTH: usize = 10000;

/// An atom as grammar.y's semantic values carry it: its text, and whether cgraph's
/// `aghtmlstr` is set on it (an HTML string, `<...>`, whose text is the inner part).
#[derive(Debug, Clone)]
struct Atom {
    text: String,
    html: bool,
}

/// An edge endpoint: the node, and its port as `concatPort` makes it (the `tailport` or
/// `headport` value, and whether it is an HTML string).
type Endpoint = (NodeId, Option<(String, bool)>);

/// An item of an edge statement (`edgelist`): a node list with its ports, or a subgraph,
/// whose nodes `endedge` takes once the whole statement is read.
#[derive(Debug, Clone)]
enum EdgeItem {
    Nodes(Vec<Endpoint>),
    Subgraph(SubgraphId),
}

/// A statement whose next `simple` is a subgraph, waiting while the subgraph's body is
/// read.
struct Suspended {
    /// The statement's items before the subgraph.
    items: Vec<EdgeItem>,
    /// Whether the enclosing body had a statement before this one (`stmtlist`).
    has_stmts: bool,
}

/// Where reading a statement stopped.
enum Step {
    /// The statement is read, up to its `optsemi`.
    Done,
    /// A subgraph's body was opened: the statement, with its items so far, waits for it.
    Open(Vec<EdgeItem>),
}

/// Attributes declared before the graph body, as set by the command line flags
/// `-Gname=value`, `-Nname=value` and `-Ename=value`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalAttrs {
    pub graph: AttrMap,
    pub node: AttrMap,
    pub edge: AttrMap,
}

pub struct Parser<'a> {
    /// Read a token at a time, as bison calls `yylex`, so the scanner's warnings come when
    /// C's do: never past the token where a syntax error stops the graph.
    lexer: Lexer<'a>,
    /// The tokens read so far; the last is [`TokenKind::Eof`] once the input has ended.
    tokens: Vec<Token>,
    pos: usize,
    graph: Graph,
    scope_stack: Vec<SubgraphId>,
    globals: GlobalAttrs,
    /// Line of the root graph's closing `}`.
    rbrace_line: usize,
    /// Keys of the last attribute list in source order: cgraph declares them in this
    /// order, which fixes their `sym->id`.
    attr_order: Vec<String>,
    /// Every item of the last attribute list, repeats included, with its value and HTML
    /// flag: `attrstmt` sets each one (`S->attrlist`).
    attr_items: Vec<(String, String, bool)>,
    /// The keys in [`Parser::attr_order`] whose first occurrence was an HTML string: the
    /// first declaration names the attribute symbol, flag included (`bindattrs`).
    html_keys: std::collections::BTreeSet<String>,
    /// cgraph's id counter for anonymous objects (`ctr` in `idmap`).
    anon_ctr: u64,
    /// Parent of each subgraph (`None`: the root graph).
    sub_parent: Vec<Option<usize>>,
    /// Whether grammar.y's `startgraph` has run (the header was read).
    started: bool,
    /// The ids of local names (`%…`) in cgraph's internal map (`aginternalmapinsert`), by
    /// object type (`g` graphs, `e` edges) and name.
    local_ids: std::collections::BTreeMap<String, u64>,
    /// The number of states on bison's stack, which [`YYMAXDEPTH`] bounds.
    yydepth: usize,
    /// bison's stack ran out (`memory exhausted`): `aagparse` returns without `graph`'s
    /// error recovery, so `agread` returns the graph as read so far.
    exhausted: bool,
}

/// Which dictionary of a graph an attribute belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DictKind {
    Graph,
    Node,
    Edge,
}

impl<'a> Parser<'a> {
    fn new(lexer: Lexer<'a>, globals: &GlobalAttrs) -> Self {
        Self {
            lexer,
            tokens: Vec::new(),
            pos: 0,
            graph: Graph::new("", GraphKind::Undirected, false),
            scope_stack: Vec::new(),
            globals: globals.clone(),
            rbrace_line: 0,
            attr_order: Vec::new(),
            attr_items: Vec::new(),
            html_keys: std::collections::BTreeSet::new(),
            anon_ctr: 1,
            sub_parent: Vec::new(),
            started: false,
            local_ids: std::collections::BTreeMap::new(),
            yydepth: 1,
            exhausted: false,
        }
    }

    /// Parses the first graph in `input`.
    pub fn parse_str(input: &'a str) -> Result<Graph> {
        let mut parser = Self::new(Lexer::new(input), &GlobalAttrs::default());
        parser.read_graph()?;
        Ok(parser.graph)
    }

    /// Parses every graph in `input`, applying `globals` before each graph body.
    pub fn parse_all(input: &'a str, globals: &GlobalAttrs) -> Result<Vec<Graph>> {
        match Self::parse_all_partial(input, globals) {
            (graphs, None) => Ok(graphs),
            (_, Some(e)) => Err(e),
        }
    }

    /// Like [`Parser::parse_all`], but keeps the graphs read before a syntax error: C's
    /// `agread` returns them one at a time, so `dot` lays out and renders those first.
    ///
    /// cgraph's scanner reads its input a line at a time (`fgets`) and each `agread`
    /// starts a new scanner, so whatever follows a graph's closing `}` on the same line
    /// is dropped, and the next graph is read from the next line, its line numbers
    /// counted from there.
    pub fn parse_all_partial(
        input: &'a str,
        globals: &GlobalAttrs,
    ) -> (Vec<Graph>, Option<GraphvizError>) {
        let mut graphs = Vec::new();
        let mut stream = GraphStream::new(input, globals);
        let mut reported = None;
        while let Some(read) = stream.next_graph(|e| reported = Some(e.clone())) {
            match read {
                Ok(g) => graphs.push(g),
                Err(e) => return (graphs, Some(e)),
            }
            // bison's stack ran out: the graph read so far, and the error.
            if let Some(e) = reported.take() {
                return (graphs, Some(e));
            }
        }
        (graphs, None)
    }

    fn peek(&mut self) -> &Token {
        self.peek_n(0)
    }

    /// The token `offset` places ahead, scanning up to it (the end of input stays the last).
    fn peek_n(&mut self, offset: usize) -> &Token {
        let want = self.pos + offset;
        while self.tokens.len() <= want
            && self.tokens.last().is_none_or(|t| t.kind != TokenKind::Eof)
        {
            self.tokens.push(self.lexer.scan());
        }
        let i = want.min(self.tokens.len() - 1);
        &self.tokens[i]
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if tok.kind != TokenKind::Eof {
            self.pos += 1;
        }
        tok
    }

    /// Shifts the lookahead if it is `kind`; any other token is a syntax error.
    fn expect(&mut self, kind: &TokenKind) -> Result<Token> {
        if &self.peek().kind == kind {
            self.shift()
        } else {
            Err(syntax_error(self.peek()))
        }
    }

    /// bison shifts the lookahead.
    fn shift(&mut self) -> Result<Token> {
        let tok = self.advance();
        self.push()?;
        Ok(tok)
    }

    /// bison reduces a rule with `rhs` symbols: it pops their states and pushes the goto.
    fn reduce(&mut self, rhs: usize) -> Result<()> {
        self.yydepth = self.yydepth.saturating_sub(rhs);
        self.push()
    }

    /// bison pushes a state (`yysetstate`). A stack of `YYMAXDEPTH` states cannot grow, and
    /// bison stops with `memory exhausted` (`yyexhaustedlab`): about 3330 nested anonymous
    /// subgraphs, 2500 with a statement before each, or 2500 edge operators in one statement.
    fn push(&mut self) -> Result<()> {
        self.yydepth += 1;
        if self.yydepth < YYMAXDEPTH {
            return Ok(());
        }
        self.exhausted = true;
        let last = self
            .tokens
            .last()
            .cloned()
            .unwrap_or_else(|| self.peek().clone());
        Err(parse_error("memory exhausted", &last))
    }

    fn is_atom_token(kind: &TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Ident(_) | TokenKind::Quoted(_) | TokenKind::Html(_)
        )
    }

    /// `atom : T_atom | qatom` and `qatom : T_qatom | qatom '+' T_qatom` (grammar.y). A
    /// name or number is `T_atom`, a quoted or HTML string `T_qatom`, so only quoted and
    /// HTML strings join with `+` (`concat`, into a plain string). The keywords `node`,
    /// `edge`, `graph`, `digraph`, `subgraph` and `strict` are their own tokens in scan.l
    /// (matched case-insensitively), not `T_atom`, so an unquoted keyword is a syntax error
    /// wherever an atom is expected (issue #29).
    fn atom(&mut self) -> Result<Atom> {
        let tok = self.peek().clone();
        let mut atom = match tok.kind {
            TokenKind::Ident(text) => {
                self.shift()?;
                self.reduce(1)?;
                return Ok(Atom { text, html: false });
            }
            TokenKind::Quoted(text) => Atom { text, html: false },
            TokenKind::Html(text) => Atom { text, html: true },
            _ => return Err(syntax_error(&tok)),
        };
        self.shift()?;
        self.reduce(1)?;
        while self.peek().kind == TokenKind::Plus {
            self.shift()?;
            let next = self.peek().clone();
            match next.kind {
                TokenKind::Quoted(s) | TokenKind::Html(s) => {
                    self.shift()?;
                    atom.html = false;
                    atom.text.push_str(&s);
                    self.reduce(3)?;
                }
                _ => return Err(syntax_error(&next)),
            }
        }
        self.reduce(1)?;
        Ok(atom)
    }

    /// `graph : hdr body | error | /* empty */`. After a syntax error, bison's recovery
    /// reduces `graph : error`, whose `graph_error` ends the scan (`aglexeof`) only when
    /// `startgraph` opened a graph; before that, bison discards tokens until the end of the
    /// input, so the scanner reads them all, and warns as it does.
    fn read_graph(&mut self) -> Result<()> {
        self.read_graph_reporting(|_| {})
    }

    /// `aagparse`: reads one graph. A syntax error goes to `report` where `aagerror`
    /// prints it; when it comes before `startgraph`, bison's `graph : error` recovery then
    /// discards the tokens to the end of the input, so the scanner's warnings for them
    /// follow the error. When bison's stack is exhausted there is no recovery: the graph
    /// read so far is kept ([`Parser::exhausted`]).
    fn read_graph_reporting(&mut self, report: impl FnOnce(&GraphvizError)) -> Result<()> {
        self.started = false;
        let result = self.parse_graph();
        if let Err(e) = &result {
            report(e);
        }
        if self.exhausted {
            self.finish_root();
            return result;
        }
        // (unless the input has already ended: the error token was the end, or `@`)
        if result.is_err()
            && !self.started
            && self.tokens.last().is_none_or(|t| t.kind != TokenKind::Eof)
        {
            while self.lexer.scan().kind != TokenKind::Eof {}
        }
        result
    }

    /// `hdr : optstrict graphtype optgraphname`, then `body`.
    fn parse_graph(&mut self) -> Result<()> {
        // optstrict : T_strict | /* empty */
        let strict = if self.peek().kind == TokenKind::Strict {
            self.shift()?;
            self.reduce(1)?;
            true
        } else {
            self.reduce(0)?;
            false
        };

        // graphtype : T_graph | T_digraph
        let kind = match self.peek().kind {
            TokenKind::Graph => GraphKind::Undirected,
            TokenKind::Digraph => GraphKind::Directed,
            _ => return Err(syntax_error(self.peek())),
        };
        self.shift()?;
        self.reduce(1)?;

        // optgraphname : atom | /* empty */. `agopen` copies the name into the new root's
        // own string dictionary, as plain text; `digraph "" {` is named, by "".
        let name = if Self::is_atom_token(&self.peek().kind) {
            let name = self.atom()?.text;
            self.reduce(1)?;
            Some(name)
        } else {
            self.reduce(0)?;
            None
        };
        self.reduce(3)?;

        // startgraph
        self.started = true;
        self.anon_ctr = if name.as_deref().is_none_or(|n| n.starts_with('%')) {
            3
        } else {
            1
        };
        self.local_ids.clear();
        if let Some(n) = name.as_ref().filter(|n| n.starts_with('%')) {
            self.local_ids.insert(format!("g{n}"), 1);
        }
        self.graph = Graph::new(name.clone().unwrap_or_default(), kind, strict);
        self.graph.empty_name = name.as_deref() == Some("");
        self.graph.graph_attrs.extend(self.globals.graph.clone());
        self.graph.node_defaults.extend(self.globals.node.clone());
        self.graph.edge_defaults.extend(self.globals.edge.clone());
        self.sub_parent.clear();
        self.graph.dicts = vec![graphviz_pure_core::AttrDicts::default()];
        // gvContext declares the node label on cgraph's ProtoGraph, which every root copies.
        self.graph.dicts[0].node.insert(
            "label".to_string(),
            graphviz_pure_core::AttrSym {
                defval: "\\N".to_string(),
                print: false,
                id: 0,
                html: false,
                html_defval: false,
                proto: true,
            },
        );
        // Command-line defaults come from cgraph's ProtoGraph dictionaries. C numbers them in
        // command-line order and these in name order; every graph copies the ProtoGraph with
        // its ids, so no lookup by id can tell the two apart.
        for (kind, map) in [
            (DictKind::Graph, self.globals.graph.clone()),
            (DictKind::Node, self.globals.node.clone()),
            (DictKind::Edge, self.globals.edge.clone()),
        ] {
            for (k, v) in map {
                let d = self.dict_mut(0, kind);
                let id = d.get(&k).map_or(d.len(), |s| s.id);
                d.insert(
                    k,
                    graphviz_pure_core::AttrSym {
                        defval: v,
                        print: false,
                        id,
                        html: false,
                        html_defval: false,
                        proto: true,
                    },
                );
            }
        }

        self.body()?;
        self.finish_root();
        // `endgraph` ends the scan here (`aglexeof`), so nothing after the closing brace is
        // read; stray tokens on the next lines are the next read's (`parse_all_partial`).
        Ok(())
    }

    /// A graph attribute first declared in a subgraph is declared on the root with
    /// default "" (agattr), so agget(root, name) is "" rather than NULL.
    fn finish_root(&mut self) {
        let declared: Vec<(String, String)> = self.graph.dicts[0]
            .graph
            .iter()
            .map(|(k, v)| (k.clone(), v.defval.clone()))
            .collect();
        for (k, v) in declared {
            self.graph.graph_attrs.entry(k).or_insert(v);
        }
    }

    /// The root's `body : '{' optstmtlist '}'`, with every subgraph body nested in it.
    ///
    /// `stmtlist : stmtlist stmt | stmt` and `stmt : attrstmt optsemi | compound optsemi`.
    /// A statement whose `simple` is a subgraph waits on [`Suspended`] while the
    /// subgraph's body is read, as it waits on bison's stack, so nesting does not recurse.
    fn body(&mut self) -> Result<()> {
        self.expect(&TokenKind::LBrace)?;
        let mut suspended: Vec<Suspended> = Vec::new();
        // Whether the body read so far has a `stmtlist` (a statement before).
        let mut has_stmts = false;
        loop {
            let step = if self.peek().kind == TokenKind::RBrace {
                // optstmtlist : stmtlist | /* empty */
                self.reduce(usize::from(has_stmts))?;
                let rbrace = self.shift()?;
                self.reduce(3)?;
                let Some(outer) = suspended.pop() else {
                    self.rbrace_line = rbrace.line;
                    return Ok(());
                };
                // subgraph : optsubghdr $@1 body; simple : subgraph
                self.reduce(3)?;
                let subg = self.closesubg();
                self.reduce(1)?;
                has_stmts = outer.has_stmts;
                let mut items = outer.items;
                items.push(EdgeItem::Subgraph(subg));
                if items.len() > 1 {
                    // $@3: getedgeitems
                    self.reduce(0)?;
                }
                self.rcompound(items)?
            } else if self.starts_stmt() {
                self.stmt()?
            } else {
                // bison reduces `optstmtlist` by default before it finds the token wrong, and
                // that push can be the one its stack has no room for.
                self.reduce(usize::from(has_stmts))?;
                return Err(syntax_error(self.peek()));
            };
            match step {
                Step::Open(items) => {
                    suspended.push(Suspended { items, has_stmts });
                    has_stmts = false;
                }
                Step::Done => {
                    // optsemi : ';' | /* empty */: a `;` only ends a statement.
                    if self.peek().kind == TokenKind::Semi {
                        self.shift()?;
                        self.reduce(1)?;
                    } else {
                        self.reduce(0)?;
                    }
                    self.reduce(2)?;
                    self.reduce(if has_stmts { 2 } else { 1 })?;
                    has_stmts = true;
                }
            }
        }
    }

    /// Whether the lookahead can start a `stmt`.
    fn starts_stmt(&mut self) -> bool {
        let kind = &self.peek().kind;
        matches!(
            kind,
            TokenKind::Graph
                | TokenKind::Node
                | TokenKind::Edge
                | TokenKind::Subgraph
                | TokenKind::LBrace
        ) || Self::is_atom_token(kind)
    }

    /// `stmt`, up to `optsemi`: `attrstmt`, or `compound : simple rcompound optattr`.
    fn stmt(&mut self) -> Result<Step> {
        match self.peek().kind {
            TokenKind::Graph | TokenKind::Node | TokenKind::Edge => {
                self.attrstmt()?;
                Ok(Step::Done)
            }
            TokenKind::Subgraph | TokenKind::LBrace => self.subgraph(Vec::new()),
            ref k if Self::is_atom_token(k) => {
                let atom = self.atom()?;
                if self.peek().kind == TokenKind::Equals {
                    // attrstmt : graphattrdefs; graphattrdefs : attrassignment
                    self.shift()?;
                    let val = self.atom()?;
                    self.reduce(3)?;
                    self.reduce(1)?;
                    self.reduce(1)?;
                    self.graph_attr_assignment(atom, val);
                    return Ok(Step::Done);
                }
                let nodes = self.nodelist(atom)?;
                // simple : nodelist
                self.reduce(1)?;
                self.rcompound(vec![EdgeItem::Nodes(nodes)])
            }
            _ => Err(syntax_error(self.peek())),
        }
    }

    /// `attrstmt(T_graph, NULL)` for `graphattrdefs : attrassignment` (`name = value`).
    fn graph_attr_assignment(&mut self, key: Atom, val: Atom) {
        self.html_keys.clear();
        if key.html {
            self.html_keys.insert(key.text.clone());
        }
        let key = key.text;
        if !self.fixed_at_root(DictKind::Graph, &key) {
            self.attr_stmt(DictKind::Graph, &key, &val.text, val.html);
            set_attr(self.current_graph_attrs_mut(), &key, val.text, val.html);
        } else if let Some(sym) = self.dict_mut(0, DictKind::Graph).get_mut(&key) {
            sym.print = true;
        }
    }

    /// `attrstmt : attrtype optmacroname attrlist`.
    fn attrstmt(&mut self) -> Result<()> {
        // attrtype : T_graph | T_node | T_edge
        let kind = match self.shift()?.kind {
            TokenKind::Graph => DictKind::Graph,
            TokenKind::Node => DictKind::Node,
            _ => DictKind::Edge,
        };
        self.reduce(1)?;
        // optmacroname : atom '=' | /* empty */
        let macro_name = if Self::is_atom_token(&self.peek().kind) {
            self.atom()?;
            self.expect(&TokenKind::Equals)?;
            self.reduce(2)?;
            true
        } else {
            self.reduce(0)?;
            false
        };
        // attrlist : optattr '[' optattrdefs ']', at least one
        self.attr_order.clear();
        self.attr_items.clear();
        self.html_keys.clear();
        let mut attrs = AttrMap::new();
        self.reduce(0)?;
        if self.peek().kind != TokenKind::LBracket {
            return Err(syntax_error(self.peek()));
        }
        self.attr_brackets(&mut attrs, true)?;
        while self.peek().kind == TokenKind::LBracket {
            self.reduce(1)?;
            self.attr_brackets(&mut attrs, true)?;
        }
        self.reduce(3)?;

        // attrstmt: "creating a macro def"
        if macro_name {
            graphviz_pure_core::diag::agwarningf("attribute macros not implemented");
        }
        // `-G`/`-N`/`-E` symbols are `fixed`: root attribute statements keep them.
        let mut kept = AttrMap::new();
        for (k, v) in &attrs {
            if k.starts_with(HTML_VALUE_PREFIX) {
                continue;
            }
            // `bindattrs` skips an edge `key`, and `attrstmt` then skips the unbound item:
            // `edge [key=k]` sets nothing.
            if kind == DictKind::Edge && k == "key" {
                continue;
            }
            if self.fixed_at_root(kind, k) {
                if let Some(sym) = self.dict_mut(0, kind).get_mut(k) {
                    sym.print = true;
                }
            } else {
                set_attr(&mut kept, k, v.clone(), attr_is_html(&attrs, k));
            }
        }
        let attrs = kept;
        // `attrstmt` runs `agattr` once per item, a repeated name included, each with its
        // own value: the last one wins, and each call gives `setattr`'s diagnostics.
        self.attr_order.clear();
        for (k, v, html) in std::mem::take(&mut self.attr_items) {
            if attrs.contains_key(&k) {
                self.attr_stmt(kind, &k, &v, html);
            }
        }
        let target = match kind {
            DictKind::Graph => self.current_graph_attrs_mut(),
            DictKind::Node => self.current_node_defaults_mut(),
            DictKind::Edge => self.current_edge_defaults_mut(),
        };
        apply_attrs(target, &attrs);
        Ok(())
    }

    /// `optattr : attrlist | /* empty */` after a node or edge statement.
    fn optattr(&mut self) -> Result<AttrMap> {
        self.attr_order.clear();
        self.html_keys.clear();
        let mut attrs = AttrMap::new();
        self.reduce(0)?;
        while self.peek().kind == TokenKind::LBracket {
            self.attr_brackets(&mut attrs, false)?;
            self.reduce(1)?;
        }
        Ok(attrs)
    }

    /// `'[' optattrdefs ']'` of `attrlist`, reduced to `attrlist`: `optattrdefs : optattrdefs
    /// attrdefs | /* empty */`, `attrdefs : attrassignment optseparator`,
    /// `attrassignment : atom '=' atom` (`appendattr`) and `optseparator : ';' | ',' | /* empty */`.
    ///
    /// `items`: also keep every item in [`Parser::attr_items`], for an attribute statement.
    fn attr_brackets(&mut self, attrs: &mut AttrMap, items: bool) -> Result<()> {
        self.expect(&TokenKind::LBracket)?;
        self.reduce(0)?;
        while Self::is_atom_token(&self.peek().kind) {
            let key = self.atom()?;
            self.expect(&TokenKind::Equals)?;
            let val = self.atom()?;
            self.reduce(3)?;
            let (key, html) = (key.text, key.html);
            if !attrs.contains_key(&key) {
                if html {
                    self.html_keys.insert(key.clone());
                }
                self.attr_order.push(key.clone());
            }
            if items {
                self.attr_items
                    .push((key.clone(), val.text.clone(), val.html));
            }
            set_attr(attrs, &key, val.text, val.html);
            if matches!(self.peek().kind, TokenKind::Comma | TokenKind::Semi) {
                self.shift()?;
                self.reduce(1)?;
            } else {
                self.reduce(0)?;
            }
            self.reduce(2)?;
            self.reduce(2)?;
        }
        self.expect(&TokenKind::RBracket)?;
        self.reduce(4)
    }

    /// `rcompound : T_edgeop simple rcompound | /* empty */` after the statement's `items`,
    /// then `optattr`, and `compound`'s action: `endedge` when there is an edge operator,
    /// else `endnode`. A subgraph as the next `simple` suspends the statement.
    fn rcompound(&mut self, mut items: Vec<EdgeItem>) -> Result<Step> {
        loop {
            // scan.l returns the other graph type's edge operator as the token '-', which no
            // rule accepts.
            let edgeop = match self.graph.kind {
                GraphKind::Directed => TokenKind::Arrow,
                GraphKind::Undirected => TokenKind::DashDash,
            };
            if self.peek().kind != edgeop {
                break;
            }
            self.shift()?;
            // $@2: getedgeitems
            self.reduce(0)?;
            if matches!(self.peek().kind, TokenKind::Subgraph | TokenKind::LBrace) {
                return self.subgraph(items);
            }
            let first = self.atom()?;
            let nodes = self.nodelist(first)?;
            self.reduce(1)?;
            items.push(EdgeItem::Nodes(nodes));
            // $@3: getedgeitems
            self.reduce(0)?;
        }
        self.reduce(0)?;
        for _ in 1..items.len() {
            self.reduce(5)?;
        }
        let attrs = self.optattr()?;
        self.reduce(3)?;
        if items.len() > 1 {
            self.endedge(&items, &attrs);
        } else {
            self.endnode(&items[0], &attrs);
        }
        Ok(Step::Done)
    }

    /// `nodelist : node | nodelist ',' node`, from the first node's atom, read already.
    fn nodelist(&mut self, first: Atom) -> Result<Vec<Endpoint>> {
        let mut nodes = vec![self.node(first)?];
        self.reduce(1)?;
        while self.peek().kind == TokenKind::Comma {
            self.shift()?;
            let atom = self.atom()?;
            nodes.push(self.node(atom)?);
            self.reduce(3)?;
        }
        Ok(nodes)
    }

    /// `node : atom | atom ':' atom | atom ':' atom ':' atom`, from its atom, read already;
    /// `appendnode` makes the node.
    fn node(&mut self, name: Atom) -> Result<Endpoint> {
        let mut port = None;
        if self.peek().kind == TokenKind::Colon {
            self.shift()?;
            let p1 = self.atom()?;
            if self.peek().kind == TokenKind::Colon {
                self.shift()?;
                let p2 = self.atom()?;
                self.reduce(5)?;
                // `concatPort` joins `port:compass` into a plain string.
                port = Some((format!("{}:{}", p1.text, p2.text), false));
            } else {
                self.reduce(3)?;
                port = Some((p1.text, p1.html));
            }
        } else {
            self.reduce(1)?;
        }
        let node_id = self.ensure_scoped_node(&name.text, name.html);
        Ok((node_id, port))
    }

    /// `subgraph : optsubghdr body`, up to the body's `{`: `optsubghdr : T_subgraph atom |
    /// T_subgraph | /* empty */`, then `opensubg`. The body is required.
    fn subgraph(&mut self, items: Vec<EdgeItem>) -> Result<Step> {
        let name = if self.peek().kind == TokenKind::Subgraph {
            self.shift()?;
            if Self::is_atom_token(&self.peek().kind) {
                let name = self.atom()?;
                self.reduce(2)?;
                Some(name)
            } else {
                self.reduce(1)?;
                None
            }
        } else {
            self.reduce(0)?;
            None
        };
        // $@1: opensubg. Its check of `SubgraphDepth` against `YYMAXDEPTH/2` never fires:
        // each level takes at least three states, so bison's stack runs out first.
        self.reduce(0)?;
        let subg = self.open_subgraph(name);
        self.scope_stack.push(subg);
        self.expect(&TokenKind::LBrace)?;
        Ok(Step::Open(items))
    }

    /// `closesubg`: the subgraph read becomes the statement's `simple`.
    fn closesubg(&mut self) -> SubgraphId {
        self.scope_stack.pop().expect("closesubg follows opensubg")
    }

    /// `endnode`: binds the node attributes, and applies them to the node list (none when
    /// the statement is a subgraph).
    fn endnode(&mut self, item: &EdgeItem, attrs: &AttrMap) {
        for k in std::mem::take(&mut self.attr_order) {
            self.bind_attr(DictKind::Node, &k);
        }
        if let EdgeItem::Nodes(nodes) = item
            && !attrs.is_empty()
        {
            for (node_id, _port) in nodes {
                apply_attrs(&mut self.graph.nodes[node_id.0 as usize].attrs, attrs);
            }
        }
    }

    /// `endedge`: binds the edge attributes, except `key`, which names the edges, and makes
    /// an edge from each tail of an item to each head of the next.
    fn endedge(&mut self, items: &[EdgeItem], attrs: &AttrMap) {
        for k in std::mem::take(&mut self.attr_order) {
            if k != "key" {
                self.bind_attr(DictKind::Edge, &k);
            }
        }
        let key = attrs
            .get("key")
            .map(|k| (k.clone(), attr_is_html(attrs, "key")));
        let mut attrs = attrs.clone();
        attrs.remove("key");
        attrs.remove(&graphviz_pure_core::html_value_key("key"));
        for pair in items.windows(2) {
            // A subgraph's nodes as they are now, after the whole statement was read.
            let tails = self.item_endpoints(&pair[0]);
            for (tail, tail_port) in tails {
                let heads = self.item_endpoints(&pair[1]);
                for (head, head_port) in heads {
                    self.newedge(
                        tail,
                        tail_port.clone(),
                        head,
                        head_port,
                        key.as_ref(),
                        &attrs,
                    );
                }
            }
        }
    }

    /// The nodes of an edge statement's item: a node list with its ports, or
    /// `agfstnode(subg)..agnxtnode` (nested subgraphs included, in node sequence order).
    fn item_endpoints(&self, item: &EdgeItem) -> Vec<Endpoint> {
        match item {
            EdgeItem::Nodes(nodes) => nodes.clone(),
            EdgeItem::Subgraph(subg) => {
                // The subgraph's list holds its nested subgraphs' nodes too.
                let mut ids = self.graph.subgraphs[subg.0 as usize].nodes.clone();
                ids.sort_by_key(|n| n.0);
                ids.into_iter().map(|nid| (nid, None)).collect()
            }
        }
    }

    /// `agsubg(S->g, name, 1)`. Only a missing name makes an anonymous subgraph; a name
    /// starting with `%` (`LOCALNAMEPREFIX`) is a local one, which also takes an anonymous
    /// id (`agmapnametoid`).
    fn open_subgraph(&mut self, name: Option<Atom>) -> SubgraphId {
        let current_parent = self.scope_stack.last().copied();
        if let Some(name) = &name
            && let Some(existing_id) = self.graph.subgraphs.iter().find_map(|s| {
                if (s.name.is_empty() && !s.empty_name)
                    || s.name != name.text
                    || s.html_name != name.html
                {
                    return None;
                }
                let has_same_parent = match current_parent {
                    Some(pid) => self.graph.subgraphs[pid.0 as usize]
                        .subgraphs
                        .contains(&s.id),
                    None => !self
                        .graph
                        .subgraphs
                        .iter()
                        .any(|p| p.subgraphs.contains(&s.id)),
                };
                has_same_parent.then_some(s.id)
            })
        {
            return existing_id;
        }

        let id = SubgraphId(self.graph.subgraphs.len() as u32);
        let named = name.is_some();
        let (name, html) = name.map_or((String::new(), false), |a| (a.text, a.html));
        let empty_name = named && name.is_empty();
        if !named {
            self.graph.anon_subgraph_ids.insert(id.0, self.anon_ctr);
            self.anon_ctr += 2;
        } else if name.starts_with('%') {
            // The root's internal map keeps the local name's id until `endgraph`: the same
            // name in another subgraph gets the same id.
            let ctr = &mut self.anon_ctr;
            let local = *self.local_ids.entry(format!("g{name}")).or_insert_with(|| {
                *ctr += 2;
                *ctr - 2
            });
            self.graph.anon_subgraph_ids.insert(id.0, local);
        }
        let is_cluster = name.to_ascii_lowercase().starts_with("cluster");
        let graph_attrs = self.current_graph_attrs().clone();
        let node_defaults = self.current_node_defaults().clone();
        let edge_defaults = self.current_edge_defaults().clone();

        self.sub_parent
            .push(self.scope_stack.last().map(|p| p.0 as usize));
        self.graph
            .dicts
            .push(graphviz_pure_core::AttrDicts::default());
        if let Some(&parent_id) = self.scope_stack.last() {
            let parent_subgraphs = &mut self.graph.subgraphs[parent_id.0 as usize].subgraphs;
            if !parent_subgraphs.contains(&id) {
                parent_subgraphs.push(id);
            }
        }

        self.graph.subgraphs.push(SubgraphData {
            id,
            name,
            is_cluster,
            graph_attrs,
            node_defaults,
            edge_defaults,
            nodes: Vec::new(),
            edges: Vec::new(),
            subgraphs: Vec::new(),
            bb: BoundingBox::default(),
            html_name: html,
            empty_name,
            layout: Default::default(),
        });

        id
    }

    /// Dictionary index of the graph in scope (0: root).
    /// `sym->fixed` in `attrstmt`: a command-line default (`global_def`) that a
    /// root-level attribute statement does not change (subgraphs still may).
    fn fixed_at_root(&self, kind: DictKind, name: &str) -> bool {
        self.scope_dict() == 0
            && match kind {
                DictKind::Graph => self.globals.graph.contains_key(name),
                DictKind::Node => self.globals.node.contains_key(name),
                DictKind::Edge => self.globals.edge.contains_key(name),
            }
    }

    fn scope_dict(&self) -> usize {
        self.scope_stack.last().map_or(0, |s| s.0 as usize + 1)
    }

    fn dict_mut(&mut self, gi: usize, kind: DictKind) -> &mut AttrMap2 {
        let d = &mut self.graph.dicts[gi];
        match kind {
            DictKind::Graph => &mut d.graph,
            DictKind::Node => &mut d.node,
            DictKind::Edge => &mut d.edge,
        }
    }

    fn dict(&self, gi: usize, kind: DictKind) -> &AttrMap2 {
        let d = &self.graph.dicts[gi];
        match kind {
            DictKind::Graph => &d.graph,
            DictKind::Node => &d.node,
            DictKind::Edge => &d.edge,
        }
    }

    /// Dictionary lookup through the view path up to the root (`agdictsym`).
    fn lookup_view(&self, gi: usize, kind: DictKind, name: &str) -> bool {
        let mut cur = Some(gi);
        while let Some(g) = cur {
            if self.dict(g, kind).contains_key(name) {
                return true;
            }
            cur = if g == 0 {
                None
            } else {
                Some(self.sub_parent[g - 1].map_or(0, |p| p + 1))
            };
        }
        false
    }

    /// `bindattrs`: an attribute unknown in scope is declared on the root with default "".
    /// For a graph attribute, `setattr` then runs `agxset_(g, rsym, "")` on the graph in
    /// scope, which gives a subgraph its own local symbol holding `""`.
    fn bind_attr(&mut self, kind: DictKind, name: &str) {
        let gi = self.scope_dict();
        if self.lookup_view(gi, kind, name) {
            return;
        }
        let html = self.html_keys.contains(name);
        let d = self.dict_mut(0, kind);
        let id = d.len();
        d.insert(
            name.to_string(),
            graphviz_pure_core::AttrSym {
                id,
                html,
                ..Default::default()
            },
        );
        if gi != 0 && kind == DictKind::Graph {
            // `agnewsym(g, sym->name, value, is_html, sym->id, …)`: the root symbol's id.
            self.dict_mut(gi, kind).insert(
                name.to_string(),
                graphviz_pure_core::AttrSym {
                    id,
                    html,
                    ..Default::default()
                },
            );
        }
    }

    /// `attrstmt` + `setattr`: a default set in the current scope.
    fn attr_stmt(&mut self, kind: DictKind, name: &str, value: &str, value_html: bool) {
        self.bind_attr(kind, name);
        let gi = self.scope_dict();
        // `setattr` warns when it updates a subgraph's own `layout` symbol. A graph
        // attribute that `bindattrs` declared from a subgraph has one already: `agattr`
        // gave the subgraph its value through `agxset`.
        if gi != 0 && name == "layout" && self.dict(gi, kind).contains_key(name) {
            graphviz_pure_core::diag::agwarningf(
                "layout attribute is invalid except on the root graph\n",
            );
        }
        // `attrstmt` names local symbols with the root symbol's name, flag included.
        let html = self.dict(0, kind).get(name).is_some_and(|s| s.html);
        if self.dict(gi, kind).contains_key(name) {
            if kind == DictKind::Graph {
                // unviewsubgraphsattr: direct children without a local value keep theirs.
                let children: Vec<usize> = if gi == 0 {
                    (0..self.sub_parent.len())
                        .filter(|&i| self.sub_parent[i].is_none())
                        .collect()
                } else {
                    self.graph.subgraphs[gi - 1]
                        .subgraphs
                        .iter()
                        .map(|c| c.0 as usize)
                        .collect()
                };
                for c in children {
                    if !self.graph.dicts[c + 1].graph.contains_key(name) {
                        let attrs = &self.graph.subgraphs[c].graph_attrs;
                        let v = attrs.get(name).cloned().unwrap_or_default();
                        let html_defval = attr_is_html(attrs, name);
                        self.graph.dicts[c + 1].graph.insert(
                            name.to_string(),
                            graphviz_pure_core::AttrSym {
                                defval: v,
                                html,
                                html_defval,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
            let sym = self.dict_mut(gi, kind).get_mut(name).unwrap();
            sym.defval = value.to_string();
            sym.html_defval = value_html;
        } else {
            self.dict_mut(gi, kind).insert(
                name.to_string(),
                graphviz_pure_core::AttrSym {
                    defval: value.to_string(),
                    html,
                    html_defval: value_html,
                    ..Default::default()
                },
            );
        }
        if gi == 0 {
            self.dict_mut(0, kind).get_mut(name).unwrap().print = true;
        }
    }

    fn ensure_scoped_node(&mut self, name: &str, html: bool) -> NodeId {
        let is_new = !self
            .graph
            .node_lookup
            .contains_key(Graph::node_key(name, html).as_ref());
        let node_id = self.graph.ensure_node_html(name, html);
        if is_new {
            let mut attrs = self.current_node_defaults().clone();
            attrs
                .entry("label".to_string())
                .or_insert_with(|| "\\N".to_string());
            let node = &mut self.graph.nodes[node_id.0 as usize];
            node.attrs = attrs;
            // A local name (`%…`) maps to an anonymous id, and `endgraph` forgets the name
            // (`aginternalmapclearlocalnames`): `agnameof` then calls the node `%<id>`.
            if name.starts_with('%') {
                node.name = format!("%{}", self.anon_ctr);
                node.html_name = false;
                self.anon_ctr += 2;
            }
        }
        self.record_node_in_active_subgraphs(node_id);
        node_id
    }

    /// `agedge(S->g, t, h, key, 1)`: the edge the statement names, or a new one; `None` where
    /// cgraph makes none.
    ///
    /// A key names one edge between the two nodes (either way round in an undirected
    /// graph), wherever in the graph it was made. Without a key, a strict graph's existing
    /// edge is taken. Otherwise a new edge is made, except in a strict graph where the
    /// current subgraph has an edge from `t` to `h` (`ok_to_make_edge`, which looks only
    /// that way round): a new key there makes nothing.
    fn agedge(&self, t: NodeId, h: NodeId, key: Option<&(String, bool)>) -> Option<(EdgeId, bool)> {
        let new = Some((EdgeId(self.graph.edges.len() as u32), true));
        if key.is_none() && !self.graph.strict {
            return new;
        }
        let undirected = self.graph.kind == GraphKind::Undirected;
        let edges = &self.graph.edges;
        // The current subgraph's edges; `None` is the root's, all of them.
        let local = self
            .scope_stack
            .last()
            .map(|s| self.graph.subgraphs[s.0 as usize].edges.as_slice());
        // `agfindedge_by_key` in `scope` for an edge from `t` to `h`, named `key` when
        // `keyed` (else any, `ok_to_make_edge`'s wildcard).
        let find = |scope: Option<&[EdgeId]>, t: NodeId, h: NodeId, keyed: bool| {
            let hit = |e: &&graphviz_pure_core::EdgeData| {
                e.tail == t
                    && e.head == h
                    && (!keyed
                        || key.is_none_or(|(k, html)| {
                            e.key.as_deref() == Some(k.as_str()) && e.key_html == *html
                        }))
            };
            match scope {
                Some(ids) => ids.iter().map(|i| &edges[i.0 as usize]).find(hit),
                None => edges.iter().find(hit),
            }
            .map(|e| e.id)
        };
        // "might already exist locally", then in the root (`subedge`)
        let scopes = if local.is_some() {
            vec![local, None]
        } else {
            vec![None]
        };
        for scope in scopes {
            let found = find(scope, t, h, true).or_else(|| {
                if undirected {
                    find(scope, h, t, true)
                } else {
                    None
                }
            });
            if let Some(e) = found {
                return Some((e, false));
            }
        }
        if self.graph.strict && find(local, t, h, false).is_some() {
            return None;
        }
        new
    }

    /// grammar.y `newedge`: `agedge`, then for the edge it gives, the endpoint ports
    /// (`mkport`) and the statement's attributes (`applyattrs`).
    fn newedge(
        &mut self,
        tail: NodeId,
        mut tail_port: Option<(String, bool)>,
        head: NodeId,
        mut head_port: Option<(String, bool)>,
        key: Option<&(String, bool)>,
        edge_attrs: &AttrMap,
    ) {
        let Some((edge_id, is_new)) = self.agedge(tail, head, key) else {
            return;
        };
        let mut merged_attrs = if is_new {
            let mut attrs = self.current_edge_defaults().clone();
            apply_attrs(&mut attrs, edge_attrs);
            let mut initial = self.graph.edge_defaults.clone();
            initial.extend(attrs.clone());
            let id = self.graph.new_edge(tail, head, initial);
            debug_assert_eq!(id, edge_id);
            let edge = &mut self.graph.edges[edge_id.0 as usize];
            if let Some((k, html)) = key {
                edge.key = Some(k.clone());
                edge.key_html = *html;
            }
            // A key draws no anonymous id, unless it is a local `%` name not seen before
            // (`aginternalmaplookup` finds the id of one that is).
            let fresh_local = key.is_some_and(|(k, _)| {
                k.starts_with('%') && self.local_ids.insert(format!("e{k}"), 0).is_none()
            });
            if key.is_none() || fresh_local {
                self.anon_ctr += 2;
            }
            attrs
        } else {
            // The defaults were applied when it was made; `applyattrs` adds this list.
            edge_attrs.clone()
        };
        let edge = &self.graph.edges[edge_id.0 as usize];
        // An existing undirected edge found the other way round takes the endpoint ports
        // swapped.
        if edge.tail != edge.head && edge.head == tail {
            std::mem::swap(&mut tail_port, &mut head_port);
        }
        // `mkport` declares the `tailport`/`headport` edge symbol (default "") the first
        // time an `n:port` endpoint is seen, tail then head, after `bindattrs` bound this
        // statement's list. Symbol ids count it at that point, which the nested flat-edge
        // layout's stale `E_arrowsz` lookup depends on.
        for (key, endpoint) in [("tailport", &tail_port), ("headport", &head_port)] {
            if endpoint.is_some() {
                self.bind_attr(DictKind::Edge, key);
            }
        }
        // `mkport` sets the endpoint ports after `agedge` applied the edge defaults and
        // before `applyattrs` applies the statement's attribute list, so an explicit
        // `tailport`/`headport` in the list wins over `n:port`, which wins over a default.
        for (key, endpoint) in [("tailport", &tail_port), ("headport", &head_port)] {
            if let Some((v, html)) = endpoint
                && !edge_attrs.contains_key(key)
            {
                set_attr(&mut merged_attrs, key, v.clone(), *html);
            }
        }
        let edge = &mut self.graph.edges[edge_id.0 as usize];
        apply_attrs(&mut edge.attrs, &merged_attrs);
        edge.tail_port = edge
            .attrs
            .get("tailport")
            .map(|s| parse_port_str(s))
            .unwrap_or_default();
        edge.head_port = edge
            .attrs
            .get("headport")
            .map(|s| parse_port_str(s))
            .unwrap_or_default();

        self.record_node_in_active_subgraphs(tail);
        self.record_node_in_active_subgraphs(head);
        self.record_edge_in_active_subgraphs(edge_id);
    }

    fn record_node_in_active_subgraphs(&mut self, node_id: NodeId) {
        for &subg_id in &self.scope_stack {
            let nodes = &mut self.graph.subgraphs[subg_id.0 as usize].nodes;
            if !nodes.contains(&node_id) {
                nodes.push(node_id);
            }
        }
    }

    fn record_edge_in_active_subgraphs(&mut self, edge_id: EdgeId) {
        for &subg_id in &self.scope_stack {
            let edges = &mut self.graph.subgraphs[subg_id.0 as usize].edges;
            if !edges.contains(&edge_id) {
                edges.push(edge_id);
            }
        }
    }

    fn current_graph_attrs(&self) -> &AttrMap {
        if let Some(&subg_id) = self.scope_stack.last() {
            &self.graph.subgraphs[subg_id.0 as usize].graph_attrs
        } else {
            &self.graph.graph_attrs
        }
    }

    fn current_graph_attrs_mut(&mut self) -> &mut AttrMap {
        if let Some(&subg_id) = self.scope_stack.last() {
            &mut self.graph.subgraphs[subg_id.0 as usize].graph_attrs
        } else {
            &mut self.graph.graph_attrs
        }
    }

    fn current_node_defaults(&self) -> &AttrMap {
        if let Some(&subg_id) = self.scope_stack.last() {
            &self.graph.subgraphs[subg_id.0 as usize].node_defaults
        } else {
            &self.graph.node_defaults
        }
    }

    fn current_node_defaults_mut(&mut self) -> &mut AttrMap {
        if let Some(&subg_id) = self.scope_stack.last() {
            &mut self.graph.subgraphs[subg_id.0 as usize].node_defaults
        } else {
            &mut self.graph.node_defaults
        }
    }

    fn current_edge_defaults(&self) -> &AttrMap {
        if let Some(&subg_id) = self.scope_stack.last() {
            &self.graph.subgraphs[subg_id.0 as usize].edge_defaults
        } else {
            &self.graph.edge_defaults
        }
    }

    fn current_edge_defaults_mut(&mut self) -> &mut AttrMap {
        if let Some(&subg_id) = self.scope_stack.last() {
            &mut self.graph.subgraphs[subg_id.0 as usize].edge_defaults
        } else {
            &mut self.graph.edge_defaults
        }
    }
}

/// flex's `YY_BUF_SIZE`, which `aagerror` names as a possible cause.
const YY_BUF_SIZE: usize = 16384;

/// The graphs of one input, read one at a time as `dot` reads them (`gvNextInputGraph`,
/// one `agread` per graph), so that each graph is laid out and rendered before the next
/// one is read.
pub struct GraphStream<'a> {
    /// Built once: every `agread` goes on reading the same stream.
    scan: ScanInput<'a>,
    /// Where the next graph starts; `None` once the input has ended or failed.
    from: Option<usize>,
    globals: GlobalAttrs,
}

impl<'a> GraphStream<'a> {
    /// The graphs of `input`, with `globals` (`-G`/`-N`/`-E`) applied before each body. The
    /// input is read as a file named by [`crate::reading_file`] around this call, else as
    /// a string.
    pub fn new(input: &'a str, globals: &GlobalAttrs) -> Self {
        GraphStream {
            scan: ScanInput::new(input),
            from: Some(0),
            globals: globals.clone(),
        }
    }

    /// The next graph, `None` at the end of the input. A syntax error is passed to
    /// `report` as soon as it is found (before the scanner reads on, as `aagerror`
    /// prints it), returned, and ends the input.
    pub fn next_graph(&mut self, report: impl FnOnce(&GraphvizError)) -> Option<Result<Graph>> {
        let from = self.from?;
        let mut parser = Parser::new(Lexer::from_scan_input(&self.scan, from), &self.globals);
        if parser.peek().kind == TokenKind::Eof {
            self.from = None;
            return None;
        }
        if let Err(e) = parser.read_graph_reporting(report) {
            // `agread` returns the graph that exhausted bison's stack, as read so far, and
            // `dot` lays it out. The next `agread` starts after the line the scanner had
            // read (`fgets`), whatever statement it is in the middle of.
            if parser.exhausted {
                let at_end = parser
                    .tokens
                    .last()
                    .is_some_and(|t| t.kind == TokenKind::Eof);
                self.from = if at_end {
                    None
                } else {
                    self.scan.line_end(from, parser.lexer.line())
                };
                return Some(Ok(parser.graph));
            }
            self.from = None;
            return Some(Err(e));
        }
        self.from = self.scan.line_end(from, parser.rbrace_line);
        Some(Ok(parser.graph))
    }
}

/// `aagerror` (`lib/cgraph/scan.l`) for bison's "syntax error" on lookahead token `tok`:
/// the line the scanner is on once `tok` is read, then `near '<yytext>'`, or, when yytext
/// is empty (end of input), the start condition the scanner was left in.
fn syntax_error(tok: &Token) -> GraphvizError {
    parse_error("syntax error", tok)
}

/// `aagerror(scanner, msg)` with `tok` the last token read.
fn parse_error(msg: &str, tok: &Token) -> GraphvizError {
    let mut message = format!("{msg} in line {}", tok.c_line);
    if !tok.text.is_empty() {
        message.push_str(&format!(" near '{}'", tok.text));
    } else {
        // `"%.80s"` of `Sbuf`: its first 80 bytes.
        let first_80 = |s: &str| {
            let bytes = graphviz_pure_core::charset::encode(s);
            graphviz_pure_core::charset::decode_slice(&bytes[..bytes.len().min(80)])
        };
        match &tok.state {
            ScanState::QString(sbuf) => {
                message.push_str(&format!(
                    " scanning a quoted string (missing endquote? longer than {YY_BUF_SIZE}?)"
                ));
                if !sbuf.is_empty() {
                    message.push_str(&format!("\nString starting:\"{}", first_80(sbuf)));
                }
            }
            ScanState::HString(sbuf) => {
                message.push_str(&format!(
                    " scanning a HTML string (missing '>'? bad nesting? longer than {YY_BUF_SIZE}?)"
                ));
                if !sbuf.is_empty() {
                    message.push_str(&format!("\nString starting:<{}", first_80(sbuf)));
                }
            }
            ScanState::Comment => message.push_str(&format!(
                " scanning a /*...*/ comment (missing '*/? longer than {YY_BUF_SIZE}?)"
            )),
            ScanState::Initial => {}
        }
    }
    GraphvizError::SyntaxError {
        line: tok.c_line,
        col: tok.col,
        message,
        file: tok.file.as_deref().map(str::to_string),
    }
}

/// A `tailport`/`headport` value (an HTML string's inner text, as `agget` gives it), split
/// at its first `:` into port name and compass point.
fn parse_port_str(s: &str) -> Port {
    if s.is_empty() {
        return Port::default();
    }
    if let Some((p1, p2)) = s.split_once(':') {
        Port {
            name: Some(p1.to_string()),
            compass: Some(p2.to_string()),
        }
    } else {
        Port {
            name: Some(s.to_string()),
            compass: None,
        }
    }
}
