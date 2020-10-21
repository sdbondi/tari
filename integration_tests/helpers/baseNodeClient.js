const expect = require('chai').expect;

class BaseNodeClient {

    constructor(client) {
        this.client = client;
    }

    getBlockTemplate() {
        return this.client.getNewBlockTemplate()
            .sendMessage({pow_algo: 1})
            .then(template => {
                return { block_reward: template.block_reward, block: template.new_block_template};
            });
    }

    submitBlockWithCoinbase(template, coinbase) {

        const cb = coinbase;
        console.log("Coinbase:", coinbase);
        template.body.outputs = template.body.outputs.concat(cb.outputs);
        template.body.kernels = template.body.kernels.concat(cb.kernels);
        console.log("Template to submit:", template);
        return this.client.getNewBlock().sendMessage(template)
            .then(b => {
                    return this.client.submitBlock().sendMessage(b.block);
                }
            );
    }

    getTipHeight() {
        return this.client.getTipInfo()
            .sendMessage({})
            .then(tip => {
                return parseInt(tip.metadata.height_of_longest_chain);
            });
    }

    mineBlock(walletClient) {
        let currHeight;
        let block;
        return this.client.getTipInfo()
            .sendMessage({})
            .then(tip => {
                currHeight = parseInt(tip.metadata.height_of_longest_chain);
                return this.client.getNewBlockTemplate()
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
                    return this.client.getNewBlock().sendMessage(block);
                }
            ).then(b => {
                    return this.client.submitBlock().sendMessage(b.block);
                }
            ).then(empty => {
                    return this.client.getTipInfo().sendMessage({});
                }
            ).then(tipInfo => {
                expect(tipInfo.metadata.height_of_longest_chain).to.equal((currHeight + 1) + "");
            });
    }
}


module.exports = BaseNodeClient;
