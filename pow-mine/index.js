async function main() {
	const worker = new Worker('./worker.js', { type: 'module' })

	const mineButton = document.getElementById('mine')
	mineButton.onclick = async () => {
		mineButton.disabled = true
		const difficulty = document.getElementById('difficulty').value
		const path = document.getElementById('path').value
		const current = document.getElementById('current').value
		const timestamp = new Date().getTime() / 1000 | 0
		worker.postMessage({ difficulty, path, current, timestamp })

		worker.onmessage = event => {
			mineButton.disabled = false
			if (event.data.ok) {
				document.getElementById('nonce').innerText = JSON.stringify(event.data.ok)
			} else {
				document.getElementById('error').innerText = event.data.err.toString()
			}
		}
	}
}

main()