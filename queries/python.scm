; Function / async function definitions (async def detected by keyword child)
(function_definition
  name: (identifier) @function.name) @function.decl

; Class definitions
(class_definition
  name: (identifier) @class.name) @class.decl

; import os, import os.path
(import_statement
  name: (dotted_name) @import.module) @import.simple

; from utils.auth import verify
(import_from_statement
  module_name: (dotted_name) @import.from) @import.from_decl

; Calls — bare identifier or attribute.identifier (obj.method())
(call
  function: [(identifier) @call.callee
             (attribute attribute: (identifier) @call.callee)]) @call.site
