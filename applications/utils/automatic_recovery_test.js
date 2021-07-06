const WalletProcess = require("../../integration_tests/helpers/walletProcess");
const WalletClient = require("../../integration_tests/helpers/walletClient");

const fs = require("fs");
const yargs = require("yargs");

async function main() {
  const argv = yargs
    .option("sw", {
      alias: "seed-words",
      description: "Seed words to use during recovery",
      type: "string",
      default:
        "pigeon marble letter canal hard close kit cash coin still melt random require long shaft antenna tent turkey neck divert enrich iron analyst abandon",
    })
    .help()
    .alias("help", "h").argv;

  const wallet = new WalletProcess(
    "compile",
    true,
    {
      transport: "tor",
      network: "weatherwax",
    },
    false,
    argv.seedWords
  );

  await wallet.startNew();

  let walletClient = new WalletClient("127.0.0.1:18143");
  let id = await walletClient.identify();

  let startTime = new Date();

  let recoveryPromise = new Promise((resolve) => {
    wallet.ps.stdout.on("data", (data) => {
      let height = data
        .toString()
        .match("Recovery\\ complete!\\ Scanned\\ =\\ (\\d+)\\ in");
      let recovered_ut = data.toString().match("worth\\ (\\d+)\\ µT");
      if (height && recovered_ut) {
        resolve({
          height: parseInt(height[1]),
          recoveredAmount: parseInt(recovered_ut[1]),
        });
      }
    });
  });

  let height_amount = await recoveryPromise;

  let endTime = new Date();
  const timeDiffMs = endTime - startTime;
  const timeDiffMinutes = timeDiffMs / 60000;
  const block_rate = height_amount.height / timeDiffMinutes;

  wallet.stop();

  console.log(
    "Wallet (Pubkey:",
    id.public_key,
    ") recovered to a block height of",
    height_amount.height,
    "completed in",
    timeDiffMinutes.toFixed(2),
    "minutes (",
    block_rate.toFixed(2),
    "blocks/min).",
    height_amount.recoveredAmount,
    "µT recovered."
  );

  fs.rmdirSync(__dirname + "/temp/base_nodes", { recursive: true });
}

Promise.all([main()]);
