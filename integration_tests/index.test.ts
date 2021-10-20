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

const buildContext = path.resolve(__dirname, '.');
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

const create = async ([url, ...urls]: string[], [main, ...rest]: Capabilities[]): Promise<S3> => {
    const hosts = await [url, ...urls].reduce(async (h, url) => {
        const hs = await h;
        const k = new Kepler(url, await zcapAuthenticator(main));
        const id = await k.new_id();
        hs[id] = [await k.id_addr(id)];
        return hs
    }, Promise.resolve({}))

    const manifest = {
        controllers: [main, ...rest].map(c => c.id()),
        hosts
    }

    console.log(manifest)

    await Promise.all(urls.map(async url => {
        const k = new Kepler(url, await zcapAuthenticator(main));
        await k.createOrbit([], { hosts: hostsToString(hosts) })
    }));

    const k = new Kepler(url, await zcapAuthenticator(main));
    const oid = await k.createOrbit([], { hosts: hostsToString(hosts) }).then(async r => await r.text());

    console.log(oid)

    return k.s3(oid)
}

const create2 = async (
    secret: string,
    [host, ...peers]: HostInfo[],
) => {
    // get IDs and multiaddrs of all kepler nodes
    const hostInfo = await Promise.all([host, ...peers].map(async host => {
        const api = "http://" + host.url + ":" + host.port;
        // @ts-ignore
        const key: string = await fetch(api + "/host").then(r => r.json()).then(({ id }) => id)
        return { [key]: ["/ip4/0.0.0.0/tcp/" + (host.port + 1)] }
    })).then(hosts => hosts.reduce((h, acc) => ({ ...h, ...acc })));

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
        hosts: hostInfo,
        admins: []
    });

    // wait for tzkt to index the contract
    await new Promise(resolve => setTimeout(resolve, 1000));

    // @ts-ignore
    const authn = await authenticator(await genClient(secret), 'test', contract);

    const hostClient = new Kepler("http://" + host.url + ":" + host.port, authn);
    const peerClients = peers.map(peer => new Kepler("http://" + peer.url + ":" + peer.port, authn));

    const hostContent = { hi: "there" };
    const peerContent = { dummy: "data" };

    const res0 = await hostClient.createOrbit(hostContent)
    const cid = await res0.text();
    expect(res0.status).toEqual(200)
    const res1 = await hostClient.resolve(cid);
    expect(res1.status).toEqual(200);
    await expect(res1.json()).resolves.toEqual(hostContent);

    await Promise.all(peerClients.map(async peerClient => {
        const createRes = await peerClient.createOrbit(peerContent);
        expect(createRes.status).toEqual(200);

        const getRes = await peerClient.resolve(cid);
        expect(getRes.status).toEqual(200);
        return await expect(getRes.json()).resolves.toEqual(hostContent);
    }))
}

describe('Kepler Integration Tests', () => {
    let environment: StartedDockerComposeEnvironment;
    let kepler1: HostInfo;
    let kepler2: HostInfo;

    beforeAll(async () => {
        // environment = await new DockerComposeEnvironment(buildContext, composeFile).up()
        // let kepler1Host = environment.getContainer("kepler-1").getHost();
        // let kepler2Host = environment.getContainer("kepler-2").getHost();
        // 10 minute time limit for building the container
    }, 600000)

    it('handles concurrent load', async () => {
        await Promise.all(secrets.map(secret => create(secret, [kepler1, kepler2])))
    }, 60000)
})
