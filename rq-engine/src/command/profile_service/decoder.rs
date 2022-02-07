use bytes::Bytes;

use crate::command::common::PbToBytes;
use crate::command::profile_service::*;
use crate::RQResult;
use crate::{pb, RQError};

impl super::super::super::Engine {
    // ProfileService.Pb.ReqSystemMsgNew.Group
    pub fn decode_system_msg_group_packet(&self, payload: Bytes) -> RQResult<GroupSystemMessages> {
        let rsp = pb::structmsg::RspSystemMsgNew::from_bytes(&payload);
        let mut join_group_requests = Vec::new();
        let mut self_invited = Vec::new();
        match rsp {
            Ok(rsp) => {
                for st in rsp
                    .groupmsgs
                    .into_iter()
                    .filter_map(|st| st.msg.map(|m| (st.msg_seq, st.req_uin, m)))
                {
                    let msg_seq = st.0;
                    let req_uin = st.1;
                    let msg = st.2;
                    match msg.sub_type {
                        // 1 进群申请
                        1 => match msg.group_msg_type {
                            1 => join_group_requests.push(JoinGroupRequest {
                                msg_seq,
                                message: msg.msg_additional,
                                req_uin,
                                req_nick: msg.req_uin_nick,
                                group_code: msg.group_code,
                                group_name: msg.group_name,
                                actor_uin: msg.actor_uin,
                                suspicious: !msg.warning_tips.is_empty(),
                                ..Default::default()
                            }),
                            2 => self_invited.push(SelfInvited {
                                msg_seq,
                                invitor_uin: msg.action_uin,
                                invitor_nick: msg.action_uin_nick,
                                group_code: msg.group_code,
                                group_name: msg.group_name,
                                actor_uin: msg.actor_uin,
                                actor_nick: msg.actor_uin_nick,
                            }),
                            22 => join_group_requests.push(JoinGroupRequest {
                                msg_seq,
                                message: msg.msg_additional,
                                req_uin,
                                req_nick: msg.req_uin_nick,
                                group_code: msg.group_code,
                                group_name: msg.group_name,
                                actor_uin: msg.actor_uin,
                                suspicious: !msg.warning_tips.is_empty(),
                                invitor_uin: Some(msg.action_uin),
                                invitor_nick: Some(msg.action_uin_qq_nick),
                            }),
                            _ => {}
                        },
                        // 2 被邀请，不需要处理
                        2 => {}
                        // ?
                        3 => {}
                        // 自身状态变更(管理员/加群退群)
                        5 => {}
                        _ => {}
                    }
                }
                Ok(GroupSystemMessages {
                    self_invited,
                    join_group_requests,
                })
            }
            Err(_) => Err(RQError::Decode(
                "failed to decode RspSystemMsgNew".to_string(),
            )),
        }
    }
}
