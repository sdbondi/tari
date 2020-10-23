const assert = require('assert');
const expect = require("chai").expect;
const grpc = require('grpc');
const protoLoader = require('@grpc/proto-loader');
const grpc_promise = require('grpc-promise');
const BaseNodeClient = require('../helpers/baseNodeClient');
const TransactionBuilder = require('../helpers/transactionBuilder');
const BaseNodeProcess = require('../helpers/baseNodeProcess');
const {sleep} = require("../helpers/util");

let client;
let walletClient;

const PROTO_PATH = __dirname + '/../../applications/tari_app_grpc/proto/base_node.proto';
const packageDefinition = protoLoader.loadSync(
    PROTO_PATH,
    {
        keepCase: true,
        longs: String,
        enums: String,
        defaults: true,
        oneofs: true
    });
const protoDescriptor = grpc.loadPackageDefinition(packageDefinition);
const tari = protoDescriptor.tari.rpc;
client = new tari.BaseNode('127.0.0.1:50051', grpc.credentials.createInsecure());
grpc_promise.promisifyAll(client);

const WALLET_PROTO_PATH = __dirname + '/../../applications/tari_app_grpc/proto/wallet.proto';
const packageDefinition2 = protoLoader.loadSync(
    WALLET_PROTO_PATH,
    {
        keepCase: true,
        longs: String,
        enums: String,
        defaults: true,
        oneofs: true
    });
const protoDescriptor2 = grpc.loadPackageDefinition(packageDefinition2);
const tariWallet = protoDescriptor2.tari.rpc;
walletClient = new tariWallet.Wallet('127.0.0.1:50061', grpc.credentials.createInsecure());
grpc_promise.promisifyAll(walletClient);

describe('Base Node', function () {
    this.timeout(10000);
    describe('As a user I want to get the version', function () {
        it('should return', function () {

            return client.getVersion()
                .sendMessage({}).then(constants => {
                    expect(constants.value).to.match(/\d+\.\d+\.\d+/);
                });
        });
    });

    describe('As a user I want to get the chain metadata', function () {
        it('should return', function () {
            return client.getTipInfo()
                .sendMessage({})
                .then(tip => {
                    expect(tip.metadata.height_of_longest_chain).to.match(/\d+/);
                })
        })
    })

    describe('As a miner I want to mine a Blake block', function () {
        it('As a miner I want to mine a Blake block', function () {
            let block;
            let curr_height;

            return client.getTipInfo()
                .sendMessage({})
                .then(tip => {
                    curr_height = parseInt(tip.metadata.height_of_longest_chain);
                    return client.getNewBlockTemplate()
                        .sendMessage({pow_algo: 1});
                })
                .then(template => {
                    block = template.new_block_template;
                    return walletClient.getCoinbase()
                        .sendMessage({
                            "reward": template.block_reward,
                            "fee": 0,
                            "height": block.header.height
                        });
                }).then(coinbase => {
                        const cb = coinbase.transaction;
                        block.body.outputs = block.body.outputs.concat(cb.body.outputs);
                        block.body.kernels = block.body.kernels.concat(cb.body.kernels);
                        return client.getNewBlock().sendMessage(block);
                    }
                ).then(b => {
                        return client.submitBlock().sendMessage(b.block);
                    }
                ).then(empty => {
                        return client.getTipInfo().sendMessage({});
                    }
                ).then(tipInfo => {
                    expect(tipInfo.metadata.height_of_longest_chain).to.equal((curr_height + 1) + "");
                });
        })
    });

    describe('As a user I want to submit a transaction' , function() {
        it('test', async function() {
            let baseNode = new BaseNodeClient(client);
            let builder = new TransactionBuilder();
            const privateKey = builder.generatePrivateKey("test");
            let blockTemplate = await baseNode.getBlockTemplate();
            let transaction = builder.generateCoinbase(blockTemplate.block_reward, privateKey, 60);
            return baseNode.submitBlockWithCoinbase(blockTemplate.block, transaction).then(async () =>
            {
                let tip = await baseNode.getTipHeight();
                console.log("Tip:", tip);
                expect(tip).to.equal(blockTemplate.block.height);
            });
        });
    });

    describe('Create a really long chain', function () {
        this.timeout(600000);
        it('describe', async function () {
            let b = new BaseNodeClient(client);
            for (var i=0;i<3000;i++) {
                await b.mineBlock(walletClient);
            }
        })

    });

    describe('Start miner and seed nodes', function() {
        this.timeout(20000);
        it("block is propagated", async function(){
            var seeds = [];
            var proc = new BaseNodeProcess();
            await proc.startNew();
            seeds.push(proc);

            var miner = new BaseNodeProcess();
            miner.setPeerSeeds([seeds[0].peerAddress()]);
            await miner.startNew();

            var minerClient = miner.createGrpcClient();
            let tip = await minerClient.getTipHeight();
            expect(tip).to.equal(0);

            var seedClient = seeds[0].createGrpcClient();
            tip = await seedClient.getTipHeight();
            expect(tip).to.equal(0);

            await minerClient.mineBlock(walletClient);
            await sleep(3000);
            expect(await minerClient.getTipHeight()).to.equal(1);

            // propagate
            await sleep(3000);
            expect(await seedClient.getTipHeight()).to.equal(1);

        });
    })
});
