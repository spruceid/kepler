#!/usr/bin/env bash
set -x

awslocal dynamodb create-table --table-name kepler-pinstore --attribute-definitions AttributeName=Cid,AttributeType=S --key-schema AttributeName=Cid,KeyType=HASH --billing-mode PAY_PER_REQUEST
awslocal dynamodb create-table --table-name kepler-indexing --attribute-definitions AttributeName=KVKey,AttributeType=S --key-schema AttributeName=KVKey,KeyType=HASH --billing-mode PAY_PER_REQUEST
awslocal s3api create-bucket --bucket kepler-blocks
