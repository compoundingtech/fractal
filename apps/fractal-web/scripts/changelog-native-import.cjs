// Vite's runner closes its config evaluator before build hooks run. Keep late imports in native
// Node, outside that evaluator's lifetime (Node 24 also strips the source registries' TS types).
module.exports = async (specifier) => import(specifier)
