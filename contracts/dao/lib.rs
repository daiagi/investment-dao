#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
pub mod dao {
    use ink::storage::Mapping;
    // use openbrush::contracts::traits::psp22::*;
    use ink::env::{
        call::{
            build_call,
            ExecutionInput,
            Selector,
        },
        DefaultEnvironment,
    };
    use scale::{
        Decode,
        Encode,
    };

    #[derive(Encode, Decode)]
    #[cfg_attr(feature = "std", derive(Debug, PartialEq, Eq, scale_info::TypeInfo))]
    pub enum VoteType {
        For,
        Against,
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum GovernorError {
        AmountShouldNotBeZero,
        DurationError,
        QuorumNotReached,
        ProposalNotAccepted,
        ProposalNotFound,
        ProposalAlreadyExecuted,
        VotePeriodExpired,
        AlreadyVoted,
        TransactionFailed,
        NotEnoughBalance,
    }

    #[derive(Encode, Decode)]
    #[cfg_attr(
        feature = "std",
        derive(
            Debug,
            PartialEq,
            Eq,
            scale_info::TypeInfo,
            ink::storage::traits::StorageLayout
        )
    )]
    pub struct Proposal {
        to: AccountId,
        vote_start: u64,
        vote_end: u64,
        executed: bool,
        amount: Balance,
    }

    #[derive(Encode, Decode, Default)]
    #[cfg_attr(
        feature = "std",
        derive(
            Debug,
            PartialEq,
            Eq,
            scale_info::TypeInfo,
            ink::storage::traits::StorageLayout
        )
    )]
    pub struct ProposalVote {
        for_votes: Balance,
        against_votes: Balance,
    }
    type ProposalId = u128;

    const ONE_MINUTE: u64 = 60;

    #[ink(storage)]
    pub struct Governor {
        proposals: Mapping<ProposalId, Proposal>,
        proposal_votes: Mapping<ProposalId, ProposalVote>,
        votes: Mapping<(ProposalId, AccountId), ()>,
        next_proposal_id: ProposalId,
        governance_token: AccountId,
        quorum: u8,
    }

    impl Governor {
        #[ink(constructor, payable)]
        pub fn new(governance_token: AccountId, quorum: u8) -> Self {
            Self {
                proposals: Mapping::new(),
                proposal_votes: Mapping::new(),
                votes: Mapping::new(),
                next_proposal_id: 0,
                governance_token,
                quorum,
            }
        }

        #[ink(message)]
        pub fn propose(
            &mut self,
            to: AccountId,
            amount: Balance,
            duration: u64,
        ) -> Result<(), GovernorError> {
            if amount == 0 {
                return Err(GovernorError::AmountShouldNotBeZero)
            }
            if duration == 0 {
                return Err(GovernorError::DurationError)
            }

            let proposal = Proposal {
                to,
                vote_start: self.env().block_timestamp(),
                vote_end: self.env().block_timestamp() + duration * ONE_MINUTE,
                executed: false,
                amount,
            };
            self.proposals.insert(self.next_proposal_id, &proposal);
            self.proposal_votes
                .insert(self.next_proposal_id, &ProposalVote::default());
            self.next_proposal_id += 1;
            Ok(())
        }

        #[ink(message)]
        pub fn vote(
            &mut self,
            proposal_id: ProposalId,
            vote: VoteType,
        ) -> Result<(), GovernorError> {
            let caller = self.env().caller();

            // Ensure the proposal exist (or return GovernorError::ProposalNotFound)
            let proposal = self
                .proposals
                .get(&proposal_id)
                .ok_or(GovernorError::ProposalNotFound)?;
            // Ensure the proposal is not executed
            if proposal.executed {
                return Err(GovernorError::ProposalAlreadyExecuted)
            }
            if proposal.vote_end < self.env().block_timestamp() {
                return Err(GovernorError::VotePeriodExpired)
            }

            // Ensure the caller has not voted yet

            if self.votes.contains(&(proposal_id, caller)) {
                return Err(GovernorError::AlreadyVoted)
            }
            // Add the caller is the votes Mapping

            self.votes.insert((proposal_id, caller), &());

            // Check the weight of the caller of the governance token (the proportion of
            // caller balance in relation to total supply)

            let caller_weight = self.get_vote_weight(caller)?;

            // Add the caller weight to the proposal vote
            let mut proposal_vote = self.proposal_votes.get(&proposal_id).unwrap();

            match vote {
                VoteType::For => {
                    proposal_vote.for_votes += caller_weight;
                }
                VoteType::Against => {
                    proposal_vote.against_votes += caller_weight;
                }
            }

            // Update the value in self.proposal_votes
            self.proposal_votes.insert(proposal_id, &proposal_vote);

            Ok(())
        }

        fn get_caller_balance(
            &self,
            caller: AccountId,
        ) -> Result<Balance, GovernorError> {
            let caller_balance = build_call::<DefaultEnvironment>()
                .call(self.governance_token)
                .gas_limit(5_000_000_000)
                .exec_input(
                    ExecutionInput::new(Selector::new(ink::selector_bytes!(
                        "PSP22::balance_of"
                    )))
                    .push_arg(caller),
                )
                .returns::<Balance>()
                .try_invoke()
                .map_err(|_| GovernorError::TransactionFailed)?
                .map_err(|_| GovernorError::TransactionFailed)?;

            Ok(caller_balance)
        }

        fn get_total_supply(&self) -> Result<Balance, GovernorError> {
            let total_supply = build_call::<DefaultEnvironment>()
                .call(self.governance_token)
                .gas_limit(5_000_000_000)
                .exec_input(ExecutionInput::new(Selector::new(ink::selector_bytes!(
                    "PSP22::total_supply"
                ))))
                .returns::<Balance>()
                .try_invoke();

            match total_supply {
                Ok(Ok(total_supply)) => Ok(total_supply),
                _ => Err(GovernorError::TransactionFailed),
            }
        }

        fn get_vote_weight(&self, account: AccountId) -> Result<Balance, GovernorError> {
            let caller_balance = self.get_caller_balance(account)?;
            let total_supply = self.get_total_supply()?;
            Ok((caller_balance * 100) / total_supply)
        }

        #[ink(message, payable)]
        pub fn transfer(
            &mut self,
            to: AccountId,
            amount: Balance,
        ) -> Result<(), GovernorError> {
            if amount > self.env().balance() {
                return Err(GovernorError::NotEnoughBalance)
            }
            match self.env().transfer(to, amount) {
                Ok(_) => Ok(()),
                Err(_err) => Err(GovernorError::TransactionFailed),
            }
        }

        #[ink(message)]
        pub fn get_proposal(&self, proposal_id: ProposalId) -> Option<Proposal> {
            self.proposals.get(&proposal_id)
        }

        #[ink(message)]
        pub fn next_proposal_id(&self) -> ProposalId {
            self.next_proposal_id
        }

        #[ink(message)]
        pub fn execute(&mut self, proposal_id: ProposalId) -> Result<(), GovernorError> {
            // Ensure the proposal exist (or returnGovernorError::ProposalNotFound)

            let proposal = self
                .proposals
                .get(&proposal_id)
                .ok_or(GovernorError::ProposalNotFound)?;

            // Ensure the proposal has not been already executed (or return
            // GovernorError::ProposalAlreadyExecuted)

            if proposal.executed {
                return Err(GovernorError::ProposalAlreadyExecuted)
            }

            // Ensure the sum of For & Against vote reach quorum (or return
            // GovernorError::QuorumNotReached)

            let proposal_vote = self.proposal_votes.get(&proposal_id).unwrap();

            let total_votes =
                (proposal_vote.for_votes + proposal_vote.against_votes) as u8;

            if total_votes < self.quorum {
                return Err(GovernorError::QuorumNotReached)
            }

            // Ensure there is more For votes than Against votes (or return
            // GovernorError::ProposalNotAccepted)

            if proposal_vote.for_votes < proposal_vote.against_votes {
                return Err(GovernorError::ProposalNotAccepted)
            }

            // Save that proposal has been executed

            let mut proposal = self.proposals.get(&proposal_id).unwrap();
            proposal.executed = true;

            // transfer amount to the recipient

            let recipient = proposal.to;
            let amount = proposal.amount;

            self.transfer(recipient, amount)
        }

        // used for test
        #[ink(message)]
        pub fn now(&self) -> u64 {
            self.env().block_timestamp()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn create_contract(initial_balance: Balance) -> Governor {
            let accounts = default_accounts();
            set_sender(accounts.alice);
            set_balance(contract_id(), initial_balance);
            Governor::new(AccountId::from([0x01; 32]), 50)
        }

        fn contract_id() -> AccountId {
            ink::env::test::callee::<ink::env::DefaultEnvironment>()
        }

        fn default_accounts(
        ) -> ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> {
            ink::env::test::default_accounts::<ink::env::DefaultEnvironment>()
        }

        fn set_sender(sender: AccountId) {
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(sender);
        }

        fn set_balance(account_id: AccountId, balance: Balance) {
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
                account_id, balance,
            )
        }

        #[ink::test]
        fn next_proposal_id_works() {
            let accounts = default_accounts();
            let mut governor = create_contract(1000);
            assert_eq!(governor.next_proposal_id(), 0);
            assert_eq!(governor.propose(accounts.django, 100, 1), Ok(()));
            assert_eq!(governor.next_proposal_id(), 1);
        }

        #[ink::test]
        fn transfer_works() {
            let mut governor = create_contract(1000);
            let recipient = AccountId::from([0x02; 32]);
            let amount = 100;

            let initial_recipient_balance = ink::env::test::get_account_balance::<
                ink::env::DefaultEnvironment,
            >(recipient)
            .unwrap_or(0);
            let initial_contract_balance = ink::env::test::get_account_balance::<
                ink::env::DefaultEnvironment,
            >(contract_id())
            .unwrap_or(0);

            assert_eq!(governor.transfer(recipient, amount), Ok(()));

            let final_recipient_balance = ink::env::test::get_account_balance::<
                ink::env::DefaultEnvironment,
            >(recipient)
            .unwrap_or(0);
            let final_contract_balance = ink::env::test::get_account_balance::<
                ink::env::DefaultEnvironment,
            >(contract_id())
            .unwrap_or(0);

            assert_eq!(final_recipient_balance, initial_recipient_balance + amount);
            assert_eq!(final_contract_balance, initial_contract_balance - amount);

            // test not enough balance
            let result = governor.transfer(recipient, 1001);
            assert_eq!(result, Err(GovernorError::NotEnoughBalance));
        }

        #[ink::test]
        fn propose_works() {
            let accounts = default_accounts();
            let mut governor = create_contract(1000);
            assert_eq!(
                governor.propose(accounts.django, 0, 1),
                Err(GovernorError::AmountShouldNotBeZero)
            );
            assert_eq!(
                governor.propose(accounts.django, 100, 0),
                Err(GovernorError::DurationError)
            );
            let result = governor.propose(accounts.django, 100, 1);
            assert_eq!(result, Ok(()));
            let proposal = governor.get_proposal(0).unwrap();
            let now = governor.now();
            assert_eq!(
                proposal,
                Proposal {
                    to: accounts.django,
                    amount: 100,
                    vote_start: 0,
                    vote_end: now + 1 * ONE_MINUTE,
                    executed: false,
                }
            );
            assert_eq!(governor.next_proposal_id(), 1);
        }

        #[ink::test]
        fn proposal_not_found() {
            let mut governor = create_contract(1000);
            let result = governor.execute(0);
            assert_eq!(result, Err(GovernorError::ProposalNotFound));
        }

        #[ink::test]
        fn quorum_not_reached() {
            let mut governor = create_contract(1000);
            let result = governor.propose(AccountId::from([0x02; 32]), 100, 1);
            assert_eq!(result, Ok(()));
            let execute = governor.execute(0);
            assert_eq!(execute, Err(GovernorError::QuorumNotReached));
        }
    }
}
