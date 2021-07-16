import { Kepler, authenticator } from 'kepler-sdk';
import { DAppClient } from '@airgap/beacon-sdk';
import { InMemorySigner } from '@taquito/signer';
import { b58cencode, prefix } from "@taquito/utils";
import { DockerComposeEnvironment, StartedDockerComposeEnvironment } from "testcontainers";
import { ContractClient } from "orbit-manifest";

import path = require('path');
const fetch = require('node-fetch');
import crypto = require('crypto');

const secrets = ['edsk3QoqBuvdamxouPhin7swCvkQNgq4jP5KZPbwWNnwdZpSpJiEbq', 'edsk3RFfvaFaxbHx8BMtEW1rKQcPtDML3LXjNqMNLCzC3wLC1bWbAt'];

const buildContext = path.resolve(__dirname, '..');
const composeFile = 'sandbox.yml';
const kepler1Port = 8000;
const kepler2Port = 9000;

const genClient = async (secret: string = b58cencode(
    crypto.randomBytes(32),
    prefix.edsk2
)): Promise<DAppClient> => {
    const ims = new InMemorySigner(secret);
    // @ts-ignore
    return {
        // @ts-ignore
        getActiveAccount: async () => ({ publicKey: await ims.publicKey(), address: await ims.publicKeyHash() }),
        // @ts-ignore
        requestSignPayload: async ({ payload }) => ({ signature: await ims.sign(payload).then(res => res.prefixSig) })
    }
}

type HostInfo = {
    id: string,
    port: number,
    url: string
}

const create = async (
    secret: string,
    host: HostInfo,
    peer: HostInfo,
) => {
    const hostAPI = host.url + ":" + host.port;
    const peerAPI = peer.url + ":" + peer.port;
    // @ts-ignore
    const { id: hk } = await fetch(hostAPI + "/hostInfo").then(async r => await r.json());
    // @ts-ignore
    const { id: pk } = await fetch(peerAPI + "/hostInfo").then(async r => await r.json());

    // deploy contract
    const client = new ContractClient({
        tzktBase: "http://localhost:5000",
        nodeURL: "http://localhost:8732",
        contractType: "",
        signer: {
            type: 'secret',
            secret
        }
    });

    const contract = await client.originate({
        hosts: {
            [hk]: [host.url + ":" + (host.port + 1)],
            [pk]: [peer.url + ":" + (peer.port + 1)],
        },
        admins: []
    });

    // @ts-ignore
    const authn = await authenticator(await genClient(secret), 'test', contract);
    const kepler1 = new Kepler(hostAPI, authn);
    const res0 = await kepler1.createOrbit({ hi: 'there' });
    expect(res0.status).toEqual(200);
    const cid = await res0.text();
    const res1 = await kepler1.resolve(cid);
    expect(res1.status).toEqual(200);
    await expect(res1.json()).resolves.toEqual({ hi: 'there' });

    const kepler2 = new Kepler(peerAPI, authn)
    const res2 = await kepler2.resolve(cid)
    expect(res2.status).toEqual(200);
    return await expect(res2.json()).resolves.toEqual({ hi: 'there' });
}

describe('Kepler Integration Tests', () => {
    let environment: StartedDockerComposeEnvironment;
    let kepler1: HostInfo;
    let kepler2: HostInfo;

    beforeAll(async () => {
        // environment = await new DockerComposeEnvironment(buildContext, composeFile).up()
        // let kepler1Host = environment.getContainer("kepler-1").getHost();
        // let kepler2Host = environment.getContainer("kepler-2").getHost();
        const kepler1Host = "http://localhost";
        const kepler2Host = "http://localhost";

        // @ts-ignore
        const { id: k1 } = await fetch(kepler1Host + ":8000" + "/hostInfo").then(async r => await r.json());
        // @ts-ignore
        const { id: k2 } = await fetch(kepler2Host + ":9000" + "/hostInfo").then(async r => await r.json());

        kepler1 = {
            url: kepler1Host,
            port: 8000,
            id: k1
        };
        kepler2 = {
            url: kepler2Host,
            port: 9000,
            id: k2
        };
        console.log(k1, k2);
        // 10 minute time limit for building the container
    }, 600000)

    afterAll(async () => {
        // await environment.down()
        // 1 minute time limit for stopping the container
    }, 60000)

    it('concurrent load', async () => {
        await Promise.all(secrets.map(secret => create(secret, kepler1, kepler2)))
    }, 60000)

    // it('concurrent load', async () => {
    //     const len = 1000
    //     const p = []
    //     for (let i = 0; i < len; i++) {
    //         p.push(create())
    //     }
    //     await Promise.all(p)
    // })
    //     const cids = await orbit.put(json1, json2);
    //     console.log(cids)

    //     // await expect(orbit.get(cid)).resolves.toEqual(json)
    //     // return await expect(orbit.del(cid)).resolves.not.toThrow()
    // })
})
