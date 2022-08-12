import { check } from 'k6';
import http from 'k6/http';
import {
    randomString,
} from 'https://jslib.k6.io/k6-utils/1.3.0/index.js';

import { setup_orbit, kepler, signer } from './utils.js';

export const options = {
    scenarios: {
        constant_request_rate: {
            executor: 'constant-arrival-rate',
            rate: 10,
            timeUnit: '1s',
            duration: '30s',
            preAllocatedVUs: 100,
        },
    },
    teardownTimeout: '3600s',
};

export function setup() {
    setup_orbit(kepler, signer);
}

export default function() {
    const key = randomString(15);
    let put_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: key, action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    let content = new ArrayBuffer(randomString(256));
    let res = http.post(`${kepler}/invoke`,
        content,
        {
            headers: put_invocation,
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);
}

export function teardown() {
    let list_invocation = http.post(`${signer}/session/invoke`,
        JSON.stringify({ name: "", action: "list" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    let res = http.post(`${kepler}/invoke`,
        null,
        {
            headers: list_invocation,
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`[TEARDOWN] ${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);
    const keys = res.json();

    for (const key of keys) {
        let del_invocation = http.post(`${signer}/session/invoke`,
            JSON.stringify({ name: key, action: "del" }),
            {
                headers: {
                    'Content-Type': 'application/json',
                },
            }).json();
        let res = http.post(`${kepler}/invoke`,
            null,
            {
                headers: del_invocation,
            }
        );
        check(res, {
            'is status 200': (r) => r.status === 200,
        });
        console.log(`[TEARDOWN] ${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);
    }
}
