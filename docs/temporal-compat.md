Notes:

* Root "out" fob is workflow
* Nested "out" fobs/funcs called from outside are update
* Nested fobs are just inline code
* Root fob out fields are queried as one query for all data
* Fob/func error is workflow/update error
* View funcs are query
* Pragma patches are patches
* Extern funcs are activities, extern fobs are child workflows
  * TODO(cretz): Could have way to mark them Nexus capable