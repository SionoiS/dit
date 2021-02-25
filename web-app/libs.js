const IPFS_API_ADDRS_KEY = 'ipfs_api_addrs'
const DEFAULT_ADDRS = 'http://localhost:5001/api/v0'

function getURL() {
    const localStorage = window.localStorage

    var url = localStorage.getItem(IPFS_API_ADDRS_KEY)

    if (url === null) {
        url = DEFAULT_ADDRS

        localStorage.setItem(IPFS_API_ADDRS_KEY, url)
    }

    return url
}

const ipfs = window.IpfsHttpClient(getURL())

export async function subscribe(topic, pubsubMessage) {
    await ipfs.pubsub.subscribe(topic, msg => pubsubMessage(msg.from, msg.data))
}

export async function publish(topic, message) {
    await ipfs.pubsub.publish(topic, message)
}

export async function unsubscribe(topic) {
    await ipfs.pubsub.unsubscribe(topic)
}

export async function nameResolve(cid) {
    for await (const path of ipfs.name.resolve(cid)) {
        return path
    }
}

export async function dagGet(cid, path) {
    const result = await ipfs.dag.get(cid, { path })

    return result.value
}

/// Get data from IPFS. Return Uint8Array
export async function cat(path) {
    let value = new Uint8Array(0)

    for await (const buf of ipfs.cat(path)) {
        const newBuf = new Uint8Array(value.length + buf.length)

        newBuf.set(value)
        newBuf.set(buf, value.length)

        value = newBuf
    }

    return value
}