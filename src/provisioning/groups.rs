//! Group management, part of the Provisioning API.

use reqwest::Method;

use crate::client::Nextcloud;
use crate::error::Result;
use crate::ocs::paged_query;
use crate::provisioning::{BASE, GroupIdList, UserIdList};

pub struct Groups<'a> {
    nc: &'a Nextcloud,
}

impl Nextcloud {
    pub fn groups(&self) -> Groups<'_> {
        Groups { nc: self }
    }
}

impl Groups<'_> {
    pub async fn list(
        &self,
        search: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<String>> {
        let query = paged_query(search, limit, offset);
        let list: GroupIdList = self
            .nc
            .ocs_typed(Method::GET, &format!("{BASE}/groups"), &query, &[])
            .await?;
        Ok(list.groups)
    }

    pub async fn create(&self, group_id: &str) -> Result<()> {
        let form = [("groupid", group_id.to_string())];
        self.nc
            .ocs_unit(Method::POST, &format!("{BASE}/groups"), &[], &form)
            .await
    }

    pub async fn members(&self, group_id: &str) -> Result<Vec<String>> {
        let list: UserIdList = self
            .nc
            .ocs_typed(Method::GET, &format!("{BASE}/groups/{group_id}"), &[], &[])
            .await?;
        Ok(list.users)
    }

    pub async fn subadmins(&self, group_id: &str) -> Result<Vec<String>> {
        self.nc
            .ocs_typed(
                Method::GET,
                &format!("{BASE}/groups/{group_id}/subadmins"),
                &[],
                &[],
            )
            .await
    }

    /// Change the group's display name.
    ///
    /// `displayname` is currently the only editable key.
    pub async fn set_display_name(&self, group_id: &str, display_name: &str) -> Result<()> {
        let form = [
            ("key", "displayname".to_string()),
            ("value", display_name.to_string()),
        ];
        self.nc
            .ocs_unit(
                Method::PUT,
                &format!("{BASE}/groups/{group_id}"),
                &[],
                &form,
            )
            .await
    }

    /// Delete a group. Members are not deleted, only their membership.
    pub async fn delete(&self, group_id: &str) -> Result<()> {
        self.nc
            .ocs_unit(
                Method::DELETE,
                &format!("{BASE}/groups/{group_id}"),
                &[],
                &[],
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_list() {
        let list: GroupIdList =
            serde_json::from_value(serde_json::json!({"groups": ["admin", "staff"]})).unwrap();
        assert_eq!(list.groups, vec!["admin", "staff"]);
    }

    #[test]
    fn group_members() {
        let list: UserIdList =
            serde_json::from_value(serde_json::json!({"users": ["alice"]})).unwrap();
        assert_eq!(list.users, vec!["alice"]);
    }

    #[test]
    fn missing_array_is_empty() {
        let list: GroupIdList = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(list.groups.is_empty());
    }
}
