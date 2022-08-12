import { check } from 'k6';
import http from 'k6/http';
import exec from 'k6/execution';
import {
    randomString,
} from 'https://jslib.k6.io/k6-utils/1.3.0/index.js';

import { setup_orbit, kepler, signer } from './utils.js';

export const options = {
    iterations: 300,
    vus: 100,
};

export default function() {
    const id = exec.scenario.iterationInTest;
    setup_orbit(kepler, signer, id);

    const key = randomString(15);
    let put_invocation = http.post(`${signer}/sessions/${id}/invoke`,
        JSON.stringify({ name: key, action: "put" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    put_invocation['Content-Type'] = 'application/json';
    let res = http.post(`${kepler}/invoke`,
        JSON.stringify({ test: "data" }),
        {
            headers: put_invocation,
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`[${id} PUT] ${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);

    let get_invocation = http.post(`${signer}/sessions/${id}/invoke`,
        JSON.stringify({ name: key, action: "get" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    get_invocation['Content-Type'] = 'application/json';
    res = http.post(`${kepler}/invoke`,
        "",
        {
            headers: get_invocation,
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`[${id} GET] ${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);

    let del_invocation = http.post(`${signer}/sessions/${id}/invoke`,
        JSON.stringify({ name: key, action: "del" }),
        {
            headers: {
                'Content-Type': 'application/json',
            },
        }).json();
    del_invocation['Content-Type'] = 'application/json';
    res = http.post(`${kepler}/invoke`,
        "",
        {
            headers: del_invocation
        }
    );
    check(res, {
        'is status 200': (r) => r.status === 200,
    });
    console.log(`[${id} DEL] ${res.headers["Spruce-Trace-Id"]} -> ${res.status}`);
}
