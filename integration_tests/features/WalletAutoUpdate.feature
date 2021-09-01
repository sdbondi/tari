@auto_update
Feature: AutoUpdate

    # Not sure why this takes so long on CI
    @long-running
    Scenario: Auto update finds a new update on console wallet
        Given I have a wallet WALLET with auto update enabled
        Then WALLET has a new software update

    Scenario: Auto update ignores update with invalid signature on console wallet
        Given I have a wallet WALLET with auto update configured with a bad signature
        And I wait 10 seconds
        Then WALLET does not have a new software update
