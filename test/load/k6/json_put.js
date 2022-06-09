import { check } from 'k6';
import http from 'k6/http';
import { b64encode } from 'k6/encoding';
import {
    randomString,
} from 'https://jslib.k6.io/k6-utils/1.3.0/index.js';

import { setup_orbit, kepler, signer } from './utils.js';

export function setup() {
    setup_orbit(kepler, signer);
}

export default function() {
    let put_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: randomString(15), action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).body;
    let data = new ArrayBuffer(b64encode(randomString(256)));
    let res = http.post(`${kepler}/invoke`,
        data,
        {
            headers: {
                // 'Content-Type': 'application/octet-stream',
                'Authorization': b64encode(put_invocation),
            },
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);
}
