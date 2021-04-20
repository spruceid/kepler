# Kepler

## Introduction

Kepler is a configurably-permissioned ~~replicating~~ content-addressed storage. Kepler storage is bucketed by Orbits, authorization policies which determine who may perform certain actions on the bucket. Orbit policies may be defined using:
  * [X] None (operations are unpermissioned)
  * [X] Public key whitelist
  * [ ] DID Verification method ID whitelist
  * [ ] Verifiable Credential requirements
  * [ ] Object Capabilities framework
  
## API

Kepler exposes a basic HTTP API with POST and GET requests for putting and reading stored entries.

### Read

#### Request
GET request format:

``` http
GET https://<host-url>/<orbit-id>/<cid>
Authorization: <auth-method-token>
```

The Authorization header value format depends on the authorization policy defined by the Orbit identified by the `orbit-id`.
Example Read request using no authorization:

``` http
GET http://localhost:8000/uAYAEHiDoN2Q6QgzD6zqWuvgFoUj130OydcuzWRl8b5q5TpWuIg/uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA
```

#### Response

Successful requests will result in a 200 response containing the fetched content. Example:

``` http
{
  "hello": "there"
}
// GET http://localhost:8000/uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA
// HTTP/1.1 200 OK
// Server: Rocket
// Date: Fri, 26 Mar 2021 13:11:34 GMT
// Transfer-Encoding: chunked
// Request duration: 0.059868s
```

### Write

Writing supports the following content types:
* [X] None: corrosponds to the Raw multicodec
* [X] `application/octet-stream`: corrosponds to the Raw multicodec
* [X] `application/json`: corrosponds to the Json multicodec
* [X] `application/msgpack`: corrosponds to the MsgPack multicodec

#### Request

POST request format:

``` http
POST https://<host-url>/<orbit-id>/
Content-Type: <content-type | none>

<content>
```

Example:
``` http
POST http://localhost:8000/uAYAEHiDoN2Q6QgzD6zqWuvgFoUj130OydcuzWRl8b5q5TpWuIg
Content-Type: application/json

{
    "hello": "hey"
}
```

Writing can also be batched using content-type `multipart/form-data`, like so:
``` http
POST http://localhost:8000/uAYAEHiDoN2Q6QgzD6zqWuvgFoUj130OydcuzWRl8b5q5TpWuIg
Content-Type: multipart/form-data; boundary=---------------------------735323031399963166993862150
Content-Length: 100

---------------------------735323031399963166993862150
Content-Disposition: form-data;
Content-Type: application/json
{
    "hello": "hey"
}

---------------------------735323031399963166993862150
Content-Disposition: form-data;
Content-Type: application/json
{
    "hello": "hey again"
}
```

#### Response

Successful requests will result in a 200 response containing the CID of the stored content. Example:

``` http
uAYAEHiDoN2Q6QgzD6zqWuvgFoUj130OydcuzWRl8b5q5TpWuIg
POST http://localhost:8000/
HTTP/1.1 200 OK
Content-Type: text/plain; charset=utf-8
Server: Rocket
Content-Length: 51
Date: Fri, 26 Mar 2021 13:12:41 GMT
Request duration: 0.058104s
```

For a batch write, the response will be a newline-delimited list of CIDs, the order of which corrosponds to the order of the multipart form-data elements. An empty line indicates a failure to write the content of that index.
