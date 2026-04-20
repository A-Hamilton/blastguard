; Function declarations
(function_declaration
  name: (identifier) @function.name) @function.decl

; Arrow-function consts — `const Foo = (x) => ...`, `const Foo =
; async () => ...`, and `const Foo = function() {...}`.
(lexical_declaration
  (variable_declarator
    name: (identifier) @fn_arrow.name
    value: [(arrow_function) (function_expression)])) @fn_arrow.decl

(variable_declaration
  (variable_declarator
    name: (identifier) @fn_arrow.name
    value: [(arrow_function) (function_expression)])) @fn_arrow.decl

; Class declarations
(class_declaration
  name: (identifier) @class.name) @class.decl

; Methods inside a class body
(method_definition
  name: (property_identifier) @method.name) @method.decl

; Import statements — source specifier
(import_statement
  source: (string (string_fragment) @import.source)) @import.decl

; CommonJS — `require('./x')` and `require("./x")`. Node.js code
; predating ES modules still uses this heavily (old Express apps,
; bench harnesses, many Node tools). Captures the string literal's
; contents and routes through the same emit_import pipeline so
; resolve_imports can promote it to an Imports edge.
(call_expression
  function: (identifier) @_require
  arguments: (arguments (string (string_fragment) @import.source))
  (#eq? @_require "require")) @import.decl

; Call expressions
(call_expression
  function: [(identifier) @call.callee (member_expression property: (property_identifier) @call.callee)]) @call.site
