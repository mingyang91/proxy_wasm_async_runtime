// The worker has its own scope and no direct access to functions/objects of the
// global scope. We import the generated JS file to make `wasm_bindgen`
// available which we need to initialize our Wasm code.
import wasm_bindgen, {startup, mine} from './pkg/pow_mine.js'

console.log('Initializing worker')

// In the worker, we have a different struct that we want to use as in
// `index.js`.
async function init_wasm_in_worker() {
    // Load the Wasm file by awaiting the Promise returned by `wasm_bindgen`.
    await wasm_bindgen('./pkg/pow_mine_bg.wasm');
		startup();

    // Set callback to handle messages passed to the worker.
    self.onmessage = event => {
        // By using methods of a struct as reaction to messages passed to the
        // worker, we can preserve our state between messages.
				const result = {};
				try {
        		result.ok = mine(event.data);
				} catch (e) {
						result.err = e
				}

        // Send response back to be handled by callback in main thread.
        self.postMessage(result);
    };
};

init_wasm_in_worker();