# jq definitions that jaq lacks, written in jq itself. They are jq's own
# definitions (builtin.jq), except `fromstream`: see `_fromstream_put`.
# `tostream`, `@csv` and `@tsv` are native; see the Rust next to this file.

# SQL-style operators
def IN(s): any(s == .; .);
def IN(src; s): any(src == s; .);
def INDEX(stream; idx_expr): reduce stream as $row ({}; .[$row | idx_expr | tostring] |= $row);
def INDEX(idx_expr): INDEX(.[]; idx_expr);
def JOIN($idx; idx_expr): [.[] | [., $idx[idx_expr]]];
def JOIN($idx; stream; idx_expr): stream | [., $idx[idx_expr]];
def JOIN($idx; stream; idx_expr; join_expr): stream | [., $idx[idx_expr]] | join_expr;

# Streams: `tostream` turns a value into events, `fromstream` builds values back
# from them, `truncate_stream` drops the first levels of every event's path.

# jq builds a value from events with a `setpath` that creates every missing
# level at once. jaq's assignments refuse to go through `null` (`null | .a = 1`
# is an error), so this creates one level at a time: `null` becomes `{}` or
# `[]` according to the key, and an array grows with `null`s up to the index.
def _fromstream_put($path; $v):
  if ($path | length) == 0 then $v
  else $path[0] as $k | $path[1:] as $rest
    | if . == null then (if $k | isnumber then [] else {} end) else . end
    | if isarray and ($k | isnumber) and $k >= length then . + [range($k - length + 1) | null] end
    | .[$k] |= _fromstream_put($rest; $v)
  end;

def fromstream(f): { x: null, e: false } as $init
  | foreach f as $i ($init;
      if .e then $init else . end
      | if $i | length == 2
        then .e = ($i[0] | length == 0) | .x |= _fromstream_put($i[0]; $i[1])
        else .e = ($i[0] | length == 1) end;
      if .e then .x else empty end);

def truncate_stream(stream): . as $n | null | stream | . as $input | if (.[0]|length) > $n then setpath([0];.[0][$n:]) else empty end;
