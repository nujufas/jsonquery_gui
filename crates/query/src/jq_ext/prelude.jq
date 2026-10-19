# jq definitions that jaq lacks, or has with another meaning, written in jq itself.
# They are jq's own definitions (builtin.jq), except `fromstream`: see `_fromstream_put`,
# and `from_entries`, which also says when a key is not a string.
# `tostream`, `delpaths`, `@csv`, `@tsv`, `@base64d`, `fromjson`, `tonumber` and `tojson` are
# native; see the Rust next to this file. A definition made after one of jaq's of the same
# name and number of arguments takes its place, but only for what is written after it:
# `with_entries`, which calls `from_entries`, has to be made again below it.

# Regular expressions: `scan` finds every match, not the first only, and gives the
# groups of a match as an array when the pattern has any.
def scan($re; $flags):
  match($re; "g" + $flags)
    | if (.captures | length) > 0 then [.captures[] | .string] else .string end;
def scan($re): scan($re; null);

# `join` writes `null` as nothing, numbers and booleans as text, and refuses an array or an
# object (jaq turned every item into text, `null` into "null").
def join($x): reduce .[] as $i (null;
    (if . == null then "" else . + $x end)
    + ($i | if type == "boolean" or type == "number" then tostring else . // "" end)
  ) // "";

# Objects and arrays of key and value. A key comes from `key`, `Key`, `name` or `Name`
# (AWS tags are `{"Key": …, "Value": …}`), and is a string: jaq made any value a key,
# and showed an object with such a key as text.
def _cut: if length > 14 then .[:11] + "..." end;
def _object_key:
  if isstring then . else error("Cannot use \(type) (\(tojson | _cut)) as object key") end;
def from_entries:
  map({(.key // .Key // .name // .Name | _object_key):
       (if has("value") then .value else .Value end)})
  | add // {};
def with_entries(f): to_entries | map(f) | from_entries;

# Deleting: every path of a `delpaths` goes in one go, not one after the other (see
# jq_ext/paths.rs); `del` is jq's own.
def delpaths($paths): _delpaths($paths);
def del(f): delpaths([path(f)]);

# `pick` is jq's own, which builds the value with `setpath` (jaq's built an object, with
# the indexes of an array as its keys).
def pick(pathexps): . as $in | reduce path(pathexps) as $p (null; setpath($p; $in | getpath($p)));

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
# level at once. This creates one level at a time: `null` becomes `{}` or
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
