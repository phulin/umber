ENTRY { author title year } {} {}
INTEGERS { count }
STRINGS { label }
MACRO { jan } { "January" }
FUNCTION { emit }
{ cite$ write$ ":" write$ title write$ newline$ }
READ
EXECUTE { skip$ }
ITERATE { emit }
REVERSE { skip$ }
SORT
