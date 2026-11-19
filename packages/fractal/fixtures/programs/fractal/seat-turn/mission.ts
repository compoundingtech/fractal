import { Fractal } from '../../../../src/effect.ts'
import factory from '../../factory/agent.ts'

// This is a normal publishable native program, not a built-in or recursive worker route.
export default Fractal.seatTurn({ meta: import.meta, reportTo: factory })
