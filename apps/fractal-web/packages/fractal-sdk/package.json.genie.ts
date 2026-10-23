import { defineCatalog, packageJson } from '../../../../repos/effect-utils/genie/external.ts'
const catalog = defineCatalog({effect:'4.0.0'})
const client = {data:{name:'@fractal/client'},meta:{workspace:{repoName:'smalltalk',memberPath:'clients/typescript/fractal-client',deps:[]}}}
const deps = catalog.compose({workspace:{repoName:'smalltalk',memberPath:'apps/fractal-web/packages/fractal-sdk'},dependencies:{workspace:[client],external:catalog.pick('effect')}})
export default packageJson({ name:'@fractal/sdk',version:'0.1.0',private:true,license:'MIT',type:'module',exports:{'./effect':'./src/effect/mod.ts'} },deps)
