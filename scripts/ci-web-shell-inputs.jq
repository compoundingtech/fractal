# Run with jq -cS --slurpfile spec nix/web-shell-inputs.json -f this-file flake.lock.
# Expand only declared inputs; lock-node names and source bookkeeping are not identity.
def nonempty_string: type == "string" and length > 0;

def node($nodes; $name):
  if ($name | nonempty_string) and ($nodes[$name] | type == "object") then
    $nodes[$name]
  else error("missing or malformed lock node: \($name)") end;

def inputs($nodes; $name):
  node($nodes; $name)
  | if has("inputs") then
      if (.inputs | type == "object") then .inputs
      else error("malformed inputs on lock node: \($name)") end
    else {} end;

def input($nodes; $name; $input):
  inputs($nodes; $name)
  | if has($input) then .[$input]
    else error("missing lock input: \($name)/\($input)") end;

# Follows paths always start at the lock root, including nested follows.
# Keep a separate path stack so follows cycles fail before graph expansion.
def resolve($nodes; $root; $ref; $following):
  if ($ref | nonempty_string) then
    node($nodes; $ref) | $ref
  elif ($ref | type == "array") and all($ref[]; nonempty_string) then
    if any($following[]; . == $ref) then
      error("cyclic lock follows: \($ref)")
    else
      reduce $ref[] as $part ($root;
        input($nodes; .; $part) as $next
        | resolve($nodes; $root; $next; $following + [$ref]))
    end
  else error("malformed lock reference: \($ref)") end;

def project($nodes; $root; $name; $ancestors):
  if any($ancestors[]; . == $name) then
    error("cyclic lock input graph: \($name)")
  else
    node($nodes; $name) as $node
    | $node.locked as $locked
    | if ($locked | type != "object")
         or ($locked.type | nonempty_string | not)
         or ($locked.narHash | nonempty_string | not)
         or (($locked | has("rev")) and ($locked.rev | nonempty_string | not)) then
        error("missing or malformed locked identity: \($name)")
      elif ($node | has("flake")) and ($node.flake | type != "boolean") then
        error("malformed flake flag: \($name)")
      else
        {
          # Preserve source-kind fields (e.g. dir), but not update timestamps.
          locked: ($locked | del(.lastModified)),
          flake: (if $node | has("flake") then $node.flake else true end),
          inputs: (inputs($nodes; $name) | to_entries | map(
            .key as $input
            | resolve($nodes; $root; .value; []) as $target
            | {key: $input, value: project($nodes; $root; $target; $ancestors + [$name])}
          ) | from_entries)
        }
      end
  end;

. as $lock
| if ($lock.nodes | type != "object") or ($lock.root | nonempty_string | not) then
    error("missing or malformed lock graph")
  elif ($spec | length) != 1 then error("expected one shell input specification")
  else $spec[0] end
| . as $shell
| if ($shell | type != "object")
     or ($shell.system | nonempty_string | not)
     or ($shell.inputs | type != "array")
     or ($shell.inputs | length == 0)
     or (all($shell.inputs[]; nonempty_string) | not) then
    error("malformed shell input specification")
  else
    {
      system: $shell.system,
      inputs: ($shell.inputs | map(
        . as $input
        | input($lock.nodes; $lock.root; $input) as $ref
        | resolve($lock.nodes; $lock.root; $ref; []) as $target
        | {key: $input, value: project($lock.nodes; $lock.root; $target; [])}
      ) | from_entries)
    }
  end
