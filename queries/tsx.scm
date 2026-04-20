; JSX component usages — `<Button ...>` and `<Button />` are calls to
; the `Button` component. HTML intrinsics (`<div>`, `<span>`) use
; lowercase names and are filtered by the `^[A-Z]` predicate.
;
; This file is compiled ONLY against the TSX tree-sitter grammar —
; the plain-TypeScript grammar (used for `.ts`, `.mts`, `.cts`) does
; not define `jsx_opening_element` / `jsx_self_closing_element` node
; types and would error with "NodeType".
(jsx_opening_element
  name: (identifier) @call.callee
  (#match? @call.callee "^[A-Z]")) @call.site

(jsx_self_closing_element
  name: (identifier) @call.callee
  (#match? @call.callee "^[A-Z]")) @call.site

; Namespaced JSX components — `<Radix.Button>`, `<UI.Card>`, or
; nested `<Radix.Dialog.Root>`. tree-sitter parses the name as a
; `member_expression` chain whose rightmost `property` is the
; component name. Capture only the tail — that's what the agent
; needs to grep for, and it aligns with how ordinary call
; expressions capture the final segment of `obj.method()`.
(jsx_opening_element
  name: (member_expression
    property: (property_identifier) @call.callee)
  (#match? @call.callee "^[A-Z]")) @call.site

(jsx_self_closing_element
  name: (member_expression
    property: (property_identifier) @call.callee)
  (#match? @call.callee "^[A-Z]")) @call.site
