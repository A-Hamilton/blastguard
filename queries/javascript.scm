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

; Call expressions
(call_expression
  function: [(identifier) @call.callee (member_expression property: (property_identifier) @call.callee)]) @call.site
