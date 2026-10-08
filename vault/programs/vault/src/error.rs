use pinocchio::error::ProgramError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum VaultError {
    InvalidVaultAccount = 1,
    InvalidTreasury = 2,
    InvalidSignature = 3,
    SelfInvocation = 4,
    InvalidInnerInstruction = 5,
    InvalidBump = 6,
}

impl From<VaultError> for ProgramError {
    fn from(error: VaultError) -> Self {
        ProgramError::Custom(error as u32)
    }
}
