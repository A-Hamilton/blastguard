; Function declarations
(function_declaration
  name: (identifier) @function.name) @function.decl

; Class declarations
(class_declaration
  name: (type_identifier) @class.name) @class.decl

; Methods inside a class body
(method_definition
  name: (property_identifier) @method.name) @method.decl

; Interfaces
(interface_declaration
  name: (type_identifier) @interface.name) @interface.decl

; Type aliases
(type_alias_declaration
  name: (type_identifier) @type_alias.name) @type_alias.decl

; Import statements — source specifier captured
(import_statement
  source: (string (string_fragment) @import.source)) @import.decl

; Call expressions — callee identifier/member name captured (used by Task 2)
(call_expression
  function: [(identifier) @call.callee (member_expression property: (property_identifier) @call.callee)]) @call.site
