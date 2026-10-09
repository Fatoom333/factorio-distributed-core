-- Test-only: allow scripts to write pickup_position / drop_position of every inserter
-- (like Bob's Adjustable Inserters does). Vanilla value is false.
for _, ins in pairs(data.raw["inserter"]) do
  ins.allow_custom_vectors = true
end
