-- Hand-written model of the external type of TwopiCircle, from Aeneas's template. Not generated:
-- scripts/proofs/extract.sh leaves this file alone.
--
-- `LEdge` is `crate::common::types::LEdge`, which the extraction was told to treat as opaque
-- (`--opaque`, recorded in extract.args): it holds `Option<TextLabel>`, `Vec<Bezier>`, ports and
-- floats, and Aeneas rejects any loop in a function that takes `&[LEdge]` next to a `&mut`
-- (docs/research/lean-experiment/extraction.md, "The #766 zero-copy kernel"). The extracted code
-- reads only `tail` and `head`, through `edge_ends` (FunsExternal.lean), so those are the fields
-- of the model.
module
public import Aeneas
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false

namespace GvpProofs.Extracted.TwopiCircle

/-- [graphviz_pure_layout::common::types::LEdge], the two fields the kernel reads. -/
structure common.types.LEdge where
  tail : Std.Usize
  head : Std.Usize

end GvpProofs.Extracted.TwopiCircle
