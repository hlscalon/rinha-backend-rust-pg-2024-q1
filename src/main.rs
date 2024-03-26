extern crate chrono;

use actix_web::{error, get, post, web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use tokio_pg_mapper_derive::PostgresMapper;
use deadpool_postgres::{Manager, ManagerConfig, Client, Pool, RecyclingMethod};
use tokio_postgres::NoTls;
use chrono::{NaiveDateTime, Local};
use std::env;

#[derive(PostgresMapper, Deserialize, Serialize)]
#[pg_mapper(table = "transacao")]
pub struct Transacao {
	pub cliente_id: i32,
	pub valor: i32,
	pub tipo: String,
	pub descricao: String,
	pub realizada_em: String,
	pub limite_atual: i32,
	pub saldo_atual: i32,
}

#[derive(PostgresMapper, Deserialize, Serialize)]
#[pg_mapper(table = "cliente")]
pub struct Cliente {
	pub nome: String,
	pub limite: i32,
	pub saldo: i32,
}

#[derive(Deserialize)]
struct TransacaoRequest {
	valor: i32,
	tipo: String,
	descricao: String
}

#[derive(Serialize)]
struct TransacaoResponse {
	saldo: i32,
	limite: i32,
}

#[derive(Serialize)]
struct TransacaoExtratoResponse {
	valor: i32,
	tipo: String,
	descricao: String,
	realizada_em: String,
}

#[derive(Serialize)]
struct SaldoExtratoResponse {
	total: i32,
	limite: i32,
	data_extrato: String,
}

#[derive(Serialize)]
struct ExtratoResponse {
	saldo: SaldoExtratoResponse,
	ultimas_transacoes: Vec<TransacaoExtratoResponse>,
}

fn is_valid_cliente_id(cliente_id: i32) -> bool {
	return cliente_id > 0 && cliente_id < 6;
}

fn get_query_transacao(tipo: String) -> String {
	let query: String;
	let select: String;

	if tipo == "c" {
		query = "UPDATE cliente SET saldo = saldo + $1 WHERE id = $2 RETURNING saldo, limite".to_string();
		select = "SELECT $3, $4, $5, $6, cliente_atualizado.limite, cliente_atualizado.saldo ".to_string();
	} else {
		query = "UPDATE cliente SET saldo = saldo - $1 WHERE id = $2 AND saldo - $3 >= -ABS(limite) RETURNING saldo, limite".to_string();
		select = "SELECT $4, $5, $6, $7, cliente_atualizado.limite, cliente_atualizado.saldo ".to_string();
	}

	return format!(
		concat!(
			"WITH cliente_atualizado AS ({}) ",
			"INSERT INTO transacao (cliente_id, valor, tipo, descricao, limite_atual, saldo_atual) ",
				"{} ",
				"FROM cliente_atualizado ",
				"RETURNING limite_atual, saldo_atual"
		),
		query, select
	);
}

#[post("/clientes/{cliente_id}/transacoes")]
async fn handle_transacao(path: web::Path<i32>, info: web::Json<TransacaoRequest>, db_pool: web::Data<Pool>) -> impl Responder {
	let cliente_id: i32 = path.into_inner();

	if !is_valid_cliente_id(cliente_id) {
		return HttpResponse::NotFound().finish();
	}

	if info.valor <= 0 {
		return HttpResponse::UnprocessableEntity().finish();
	}

	if info.tipo != "c" && info.tipo != "d" {
		return HttpResponse::UnprocessableEntity().finish();
	}

	if info.descricao == "" {
		return HttpResponse::UnprocessableEntity().finish();
	}

	let descricao_len: usize = info.descricao.len();
	if descricao_len < 1 || descricao_len > 10 {
		return HttpResponse::UnprocessableEntity().finish();
	}

	let db_client: Client = db_pool.get().await.unwrap();
	let query: String = get_query_transacao(info.tipo.clone());
	let stmt = db_client.prepare(&query).await.unwrap();
	let mut saldo_atual: i32 = 0;
	let mut limite_atual: i32 = 0;

	if info.tipo == "c" {
		db_client.query(&stmt, &[
			&info.valor,
			&cliente_id,
			&cliente_id,
			&info.valor,
			&info.tipo,
			&info.descricao,
		])
		.await
		.unwrap()
		.iter()
		.for_each(|row| {
			saldo_atual = row.get::<&str, i32>("saldo_atual");
			limite_atual = row.get::<&str, i32>("limite_atual");
		});
	} else {
		db_client.query(&stmt, &[
			&info.valor,
			&cliente_id,
			&info.valor,
			&cliente_id,
			&info.valor,
			&info.tipo,
			&info.descricao,
		])
		.await
		.unwrap()
		.iter()
		.for_each(|row| {
			saldo_atual = row.get::<&str, i32>("saldo_atual");
			limite_atual = row.get::<&str, i32>("limite_atual");
		});
	}

	let response = TransacaoResponse {
		saldo: saldo_atual,
		limite: limite_atual,
	};

	return HttpResponse::Ok().json(web::Json(response));
}

#[get("/clientes/{cliente_id}/extrato")]
async fn handle_extrato(path: web::Path<i32>, db_pool: web::Data<Pool>) -> impl Responder {
	let cliente_id: i32 = path.into_inner();

	if !is_valid_cliente_id(cliente_id) {
		return HttpResponse::NotFound().finish();
	}

	let db_client: Client = db_pool.get().await.unwrap();
	let query: String = format!(
		concat!(
			"SELECT valor, tipo, descricao, realizada_em, limite_atual, saldo_atual ",
			"FROM transacao ",
			"WHERE cliente_id = $1 ",
			"ORDER BY id DESC ",
			"LIMIT 11 " // Deve pegar uma a mais para ignorar a inicial depois, se necessário"
		)
	);
	let stmt = db_client.prepare(&query).await.unwrap();

	let mut limite_atual: Option<i32> = None;
	let mut saldo_atual: Option<i32> = None;

	let mut ultimas_transacoes = db_client
		.query(&stmt, &[
			&cliente_id,
		])
		.await
		.unwrap()
		.iter()
		.map(|row| {
			if limite_atual == None {
				limite_atual = Some(row.get::<&str, i32>("limite_atual"));
			}

			if saldo_atual == None {
				saldo_atual = Some(row.get::<&str, i32>("saldo_atual"));
			}

			let realizada_em: NaiveDateTime = row.get::<&str, NaiveDateTime>("realizada_em");

			return TransacaoExtratoResponse {
				descricao: row.get::<&str, String>("descricao"),
				tipo: row.get::<&str, String>("tipo"),
				valor: row.get::<&str, i32>("valor"),
				realizada_em: realizada_em.to_string(),
			};
		})
		.collect::<Vec<TransacaoExtratoResponse>>();

	if ultimas_transacoes.is_empty() {
		return HttpResponse::UnprocessableEntity().finish();
	}

	ultimas_transacoes.pop();
	let date = Local::now();

	let response = ExtratoResponse {
		saldo: SaldoExtratoResponse {
			total: saldo_atual.expect("saldo atual"),
			limite: limite_atual.expect("limite atual"),
			data_extrato: date.to_string(), 
		},
		ultimas_transacoes: ultimas_transacoes
	};

	return HttpResponse::Ok().json(web::Json(response));
}

async fn get_connection() -> Pool {
	let mut pg_config = tokio_postgres::Config::new();

	pg_config.host(env::var("DB_HOST").unwrap().as_str());
	pg_config.user(env::var("DB_USER").unwrap().as_str());
	pg_config.password(env::var("DB_PASSWORD").unwrap().as_str());
	pg_config.dbname(env::var("DB_NAME").unwrap().as_str());
	pg_config.port(env::var("DB_PORT").unwrap().as_str().parse().unwrap());

	let manager_config = ManagerConfig {
		recycling_method: RecyclingMethod::Fast
	};
	
	let manager = Manager::from_config(pg_config.clone(), NoTls, manager_config.clone());
	return Pool::builder(manager).max_size(10).build().unwrap();
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	env_logger::init();

	let server_port: u16 = env::var("SERVER_PORT").unwrap().as_str().parse().unwrap();
	let pool = get_connection().await;

	let server =
		HttpServer::new(move || {
			let json_config = web::JsonConfig::default()
				.limit(256)
				.error_handler(|err, _req| {
					return error::InternalError::from_response(err, HttpResponse::UnprocessableEntity().finish()).into()
				});

			App::new()
				.app_data(web::Data::new(pool.clone()))
				.app_data(json_config)
				.service(handle_transacao)
				.service(handle_extrato)
		})
		.bind(("0.0.0.0", server_port))?
		.run();

	println!("Servidor rodando na porta: {}", server_port);

	return server.await;
}
