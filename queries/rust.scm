; Function declarations
(function_item
  name: (identifier) @function.name) @function.decl

; Structs
(struct_item
  name: (type_identifier) @struct.name) @struct.decl

; Enums
(enum_item
  name: (type_identifier) @enum.name) @enum.decl

; Traits
(trait_item
  name: (type_identifier) @trait.name) @trait.decl

; use declarations — capture the full argument (path or path::*).
(use_declaration
  argument: (_) @use.path) @use.decl

; Calls — identifier, field access, or scoped path.
(call_expression
  function: [(identifier) @call.callee
             (field_expression field: (field_identifier) @call.callee)
             (scoped_identifier)]) @call.site
